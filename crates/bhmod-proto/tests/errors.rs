use bhmod_proto::cbor::{encode, Value};
use bhmod_proto::*;

fn packet(op: u8, body: Value) -> Vec<u8> {
    let mut p = vec![PACKET_TYPE, op];
    p.extend(encode(&body));
    p
}

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(entries.into_iter().map(|(k, v)| (Value::text(k), v)).collect())
}

fn err(p: &[u8]) -> Error {
    Message::decode(p).unwrap_err()
}

#[test]
fn display() {
    let all = [
        (Error::NotModPacket, "not a 0xBC packet"),
        (Error::Truncated, "truncated packet"),
        (Error::UnknownOp(9), "unknown sub-opcode 0x09"),
        (Error::Cbor("x"), "bad CBOR: x"),
        (Error::Missing("k"), "missing key `k`"),
        (Error::Type("k"), "wrong type or range for `k`"),
        (Error::TooLarge("body"), "body too large"),
        (Error::NotMap, "body is not a CBOR map"),
        (Error::Invalid("why"), "invalid message: why"),
    ];
    for (e, s) in all {
        assert_eq!(e.to_string(), s);
    }
    let _: &dyn std::error::Error = &Error::NotMap;
}

#[test]
fn size_limits() {
    let mut chunk = vec![PACKET_TYPE, 0x05];
    chunk.extend([0u8; 32]);
    chunk.extend(0u64.to_le_bytes());
    chunk.extend(u64::MAX.to_le_bytes());
    chunk.extend(vec![0u8; CHUNK_SIZE + 1]);
    assert_eq!(err(&chunk), Error::TooLarge("chunk"));

    let mut ch = vec![PACKET_TYPE, 0x10, 1, 0];
    ch.extend(vec![0u8; MAX_CHANNEL_PAYLOAD + 1]);
    assert_eq!(err(&ch), Error::TooLarge("channel payload"));

    let mut big = vec![PACKET_TYPE, 0x02];
    big.extend(vec![0u8; MAX_BODY + 1]);
    assert_eq!(err(&big), Error::TooLarge("body"));

    let call = |args: Vec<u8>| {
        packet(
            0x13,
            map(vec![
                ("id", Value::Uint(1)),
                ("mod", Value::text("a")),
                ("service", Value::text("s")),
                ("req", Value::text("*")),
                ("method", Value::text("m")),
                ("args", Value::Bytes(args)),
            ]),
        )
    };
    assert!(Message::decode(&call(vec![0x80; MAX_SERVICE_PAYLOAD])).is_ok());
    assert_eq!(err(&call(vec![0x80; MAX_SERVICE_PAYLOAD + 1])), Error::TooLarge("service payload"));
    let result = packet(
        0x14,
        map(vec![
            ("id", Value::Uint(1)),
            ("ok", Value::Bool(true)),
            ("value", Value::Bytes(vec![0; MAX_SERVICE_PAYLOAD + 1])),
        ]),
    );
    assert_eq!(err(&result), Error::TooLarge("service payload"));
}

#[test]
fn wrong_types() {
    let hello = |k: &str, v: Value| {
        let mut e = vec![
            ("proto", Value::Uint(1)),
            ("loader", Value::text("0.3.0")),
            ("platform", Value::text("x")),
            ("api", Value::text("0.1.0")),
            ("runtimes", Value::Array(vec![])),
            ("cached", Value::Array(vec![])),
        ];
        e.retain(|(name, _)| *name != k);
        e.push((k, v));
        packet(0x00, map(e))
    };
    assert_eq!(err(&hello("runtimes", Value::Array(vec![Value::Uint(1)]))), Error::Type("runtimes"));
    assert_eq!(err(&hello("runtimes", Value::text("wasm"))), Error::Type("runtimes"));
    assert_eq!(err(&hello("cached", Value::Array(vec![Value::text("x")]))), Error::Type("cached"));
    assert_eq!(err(&hello("proto", Value::text("1"))), Error::Type("proto"));

    let sh = map(vec![
        ("proto", Value::Uint(1)),
        ("modsHash", Value::Bytes(vec![0; 32])),
        ("clientRequired", Value::Uint(1)),
        ("count", Value::Uint(0)),
    ]);
    assert_eq!(err(&packet(0x01, sh)), Error::Type("clientRequired"));

    let list = |mods: Value, registry: Value| {
        packet(0x03, map(vec![("mods", mods), ("channels", Value::Array(vec![])), ("registry", registry)]))
    };
    assert_eq!(err(&list(Value::Array(vec![Value::Uint(1)]), Value::Map(vec![]))), Error::Type("mods"));
    let reg = Value::Map(vec![(Value::text("blocks"), Value::Map(vec![(Value::text("a:b"), Value::text("1"))]))]);
    assert_eq!(err(&list(Value::Array(vec![]), reg)), Error::Type("registry"));
    let reg = Value::Map(vec![(Value::Uint(1), Value::Map(vec![]))]);
    assert_eq!(err(&list(Value::Array(vec![]), reg)), Error::Type("registry"));
    let reg = Value::Map(vec![(Value::text("blocks"), Value::Map(vec![(Value::Uint(1), Value::Uint(1))]))]);
    assert_eq!(err(&list(Value::Array(vec![]), reg)), Error::Type("registry"));
    assert_eq!(err(&list(Value::Array(vec![]), Value::Array(vec![]))), Error::Type("registry"));

    let rt = map(vec![("reasonCode", Value::Uint(3)), ("text", Value::text("x")), ("url", Value::Uint(1))]);
    assert_eq!(err(&packet(0x08, rt)), Error::Type("url"));
    let fr = map(vec![("sha256", Value::Uint(0)), ("offset", Value::Uint(0))]);
    assert_eq!(err(&packet(0x04, fr)), Error::Type("sha256"));
}

#[test]
fn unknown_keys_are_ignored() {
    let fr = map(vec![("sha256", Value::Bytes(vec![7; 32])), ("offset", Value::Uint(5)), ("future", Value::Null)]);
    assert_eq!(
        Message::decode(&packet(0x04, fr)).unwrap(),
        Message::FileRequest(FileRequest { sha256: [7; 32], offset: 5 })
    );
}

#[test]
fn constants() {
    assert_eq!(vanilla_reject_packet(), [0x26, 3]);
    assert_eq!(PLIST_KEY, "bhcModLoader");
    assert_eq!(HELLO_TIMEOUT_MS, 1500);
    assert_eq!(MAX_IN_FLIGHT, 8);
    assert_eq!(
        service_error::NOT_FOUND
            + service_error::VERSION_MISMATCH
            + service_error::NOT_PERMITTED
            + service_error::BUSY
            + service_error::PROVIDER_ERROR
            + service_error::TRAP
            + service_error::BAD_VALUE
            + service_error::TOO_LARGE
            + service_error::INTERNAL,
        45
    );
    assert_eq!(MAX_SERVICE_PAYLOAD, 32 * 1024);
    assert_eq!(file_error::BAD_OFFSET + file_error::BUSY + file_error::INTERNAL, 9);
}

#[test]
fn voice_and_settings_limits() {
    let mut frame = vec![PACKET_TYPE, 0x16];
    frame.extend(5u64.to_le_bytes());
    frame.extend(vec![0u8; MAX_VOICE_SEALED]);
    assert!(Message::decode(&frame).unwrap().is_unreliable());
    frame.push(0);
    assert_eq!(err(&frame), Error::TooLarge("voice frame"));

    let schema = |schema: Vec<u8>, values: Vec<u8>| {
        packet(
            0x19,
            map(vec![("mod", Value::text("m")), ("schema", Value::Bytes(schema)), ("values", Value::Bytes(values))]),
        )
    };
    assert!(Message::decode(&schema(vec![0xa0; MAX_SETTINGS_PAYLOAD], vec![0xa0])).is_ok());
    assert_eq!(err(&schema(vec![0xa0; MAX_SETTINGS_PAYLOAD + 1], vec![0xa0])), Error::TooLarge("schema"));
    assert_eq!(err(&schema(vec![0xa0], vec![0xa0; MAX_SETTINGS_PAYLOAD + 1])), Error::TooLarge("values"));
    let set = packet(
        0x1a,
        map(vec![
            ("mod", Value::text("m")),
            ("key", Value::text("k")),
            ("value", Value::Bytes(vec![0; MAX_SETTINGS_VALUE + 1])),
        ]),
    );
    assert_eq!(err(&set), Error::TooLarge("value"));
    let result = packet(0x1b, map(vec![("ok", Value::Bool(false)), ("code", Value::Uint(256))]));
    assert_eq!(err(&result), Error::Type("code"));
}

#[test]
fn voice_state_maps_are_checked() {
    let state = |extra: (&str, Value)| {
        packet(0x17, map(vec![("full", Value::Bool(false)), ("players", Value::Map(vec![])), extra]))
    };
    assert_eq!(err(&state(("speakers", Value::Uint(1)))), Error::Type("speakers"));
    let big = Value::Map(vec![(Value::text("p"), Value::Uint(70000))]);
    assert_eq!(err(&state(("speakers", big))), Error::Type("speakers"));
    let key = Value::Map(vec![(Value::Uint(1), Value::Uint(1))]);
    assert_eq!(err(&state(("speakers", key))), Error::Type("speakers"));
    let Message::VoiceState(s) = Message::decode(&state(("other", Value::Null))).unwrap() else { panic!() };
    assert!(s.speakers.is_empty() && s.icons.is_empty() && !s.full);
}

#[test]
fn reliability_per_op() {
    for b in 0..=0xffu8 {
        if let Some(op) = Op::from_u8(b) {
            assert_eq!(op as u8, b);
            assert_eq!(op.is_unreliable(), b == 0x16);
        }
    }
    assert!(!Message::ListRequest.is_unreliable());
}
