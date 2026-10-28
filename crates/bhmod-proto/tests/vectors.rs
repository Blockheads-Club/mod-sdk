//! Byte-exact test vectors in `vectors/`. Every implementation of the 0xBC protocol (server, PC loader, Android
//! loader) must encode these messages to exactly these bytes and decode the bytes back to the same fields.
//!
//! Regenerate after an intentional change with `BHMOD_BLESS=1 cargo test -p bhmod-proto --test vectors`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use bhmod_proto::cbor::{self, Value};
use bhmod_proto::*;
use serde_json::{json, Value as Json};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vectors")
}

fn bless() -> bool {
    std::env::var_os("BHMOD_BLESS").is_some()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn from_hex(s: &str) -> Vec<u8> {
    let s = s.trim();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// sha256 of a short ASCII label, so the vectors use realistic-looking but reproducible hashes.
fn h(label: &str) -> Hash {
    use sha2::Digest;
    sha2::Sha256::digest(label.as_bytes()).into()
}

/// CBOR value -> JSON for the human readable `vectors.json` (byte strings as `{"$bytes": "<hex>"}`).
fn cbor_json(v: &Value) -> Json {
    match v {
        Value::Uint(n) => json!(n),
        Value::Bytes(b) => json!({ "$bytes": to_hex(b) }),
        Value::Text(s) => json!(s),
        Value::Array(a) => Json::Array(a.iter().map(cbor_json).collect()),
        Value::Map(m) => Json::Object(
            m.iter()
                .map(|(k, v)| match k {
                    Value::Text(k) => (k.clone(), cbor_json(v)),
                    other => panic!("non-text key {other:?}"),
                })
                .collect(),
        ),
        Value::Bool(b) => json!(b),
        Value::Null => Json::Null,
    }
}

fn landclaim() -> ModInfo {
    ModInfo {
        id: "landclaim".into(),
        version: "1.4.0".into(),
        name: BTreeMap::from([("en".into(), "Land Claim".into()), ("tr".into(), "Arazi Sahipliği".into())]),
        description: BTreeMap::from([("en".into(), "Claim and protect land with signs.".into())]),
        sides: vec!["server".into(), "client".into()],
        client_required: true,
        sha256: h("landclaim-1.4.0.bhmod"),
        size: 48213,
        permissions: ModPermissions {
            world: "write".into(),
            players: vec!["chat".into(), "teleport".into()],
            net_http: vec![],
            channels: true,
            native: false,
        },
        source_url: Some("https://github.com/someone/landclaim/releases/download/v1.4.0/landclaim-1.4.0.bhmod".into()),
        index_listed: true,
    }
}

fn hello_lua() -> ModInfo {
    ModInfo {
        id: "hello-lua".into(),
        version: "0.1.0".into(),
        name: BTreeMap::from([("en".into(), "Hello (Lua)".into())]),
        description: BTreeMap::new(),
        sides: vec!["server".into()],
        client_required: false,
        sha256: h("hello-lua-0.1.0.bhmod"),
        size: 1532,
        permissions: ModPermissions { world: "none".into(), players: vec!["chat".into()], ..Default::default() },
        source_url: None,
        index_listed: false,
    }
}

/// (file stem, direction, description, message)
fn valid() -> Vec<(&'static str, &'static str, &'static str, Message)> {
    let landclaim_hash = mods_hash([("landclaim", "1.4.0", &h("landclaim-1.4.0.bhmod"))]);
    let chunk0: Vec<u8> = (0..CHUNK_SIZE).map(|i| (i % 251) as u8).collect();
    vec![
        (
            "00-hello",
            "C->S",
            "Modded Android client, nothing cached.",
            Message::Hello(Hello {
                proto: 1,
                loader: "0.3.0".into(),
                platform: "android-armv7".into(),
                api: "0.1.0".into(),
                runtimes: vec!["wasm".into(), "lua".into()],
                cached: vec![],
                features: vec![],
            }),
        ),
        (
            "00-hello-cached",
            "C->S",
            "Modded PC client with one package in its cache.",
            Message::Hello(Hello {
                proto: 1,
                loader: "0.3.0".into(),
                platform: "pc-linux-x86_64".into(),
                api: "0.1.0".into(),
                runtimes: vec!["wasm".into(), "lua".into(), "jvm".into()],
                cached: vec![h("landclaim-1.4.0.bhmod")],
                features: vec![],
            }),
        ),
        (
            "00-hello-features",
            "C->S",
            "Modded PC client that speaks the voice and settings ops.",
            Message::Hello(Hello {
                proto: 1,
                loader: "0.4.0".into(),
                platform: "pc-linux-x86_64".into(),
                api: "0.4.0".into(),
                runtimes: vec!["wasm".into(), "lua".into()],
                cached: vec![],
                features: vec![feature::VOICE.into(), feature::SETTINGS.into()],
            }),
        ),
        (
            "01-server-hello",
            "S->C",
            "Server with one client-required mod (landclaim 1.4.0).",
            Message::ServerHello(ServerHello {
                proto: 1,
                mods_hash: landclaim_hash,
                client_required: true,
                count: 2,
                players: None,
                features: vec![],
                voice_udp_port: None,
            }),
        ),
        (
            "01-server-hello-server-only",
            "S->C",
            "Server whose mods are all server-only: empty modsHash set.",
            Message::ServerHello(ServerHello {
                proto: 1,
                mods_hash: mods_hash([]),
                client_required: false,
                count: 1,
                players: None,
                features: vec![],
                voice_udp_port: None,
            }),
        ),
        (
            "01-server-hello-players",
            "S->C",
            "Server with the optional player count (3 of 16 players).",
            Message::ServerHello(ServerHello {
                proto: 1,
                mods_hash: mods_hash([]),
                client_required: false,
                count: 1,
                players: Some(PlayerCount { online: 3, capacity: 16 }),
                features: vec![],
                voice_udp_port: None,
            }),
        ),
        (
            "01-server-hello-features",
            "S->C",
            "Server speaking the voice and settings ops, with a dedicated voice UDP port.",
            Message::ServerHello(ServerHello {
                proto: 1,
                mods_hash: mods_hash([]),
                client_required: false,
                count: 1,
                players: None,
                features: vec![feature::VOICE.into(), feature::SETTINGS.into()],
                voice_udp_port: Some(15152),
            }),
        ),
        ("02-list-request", "C->S", "Empty map body.", Message::ListRequest),
        (
            "03-mod-list",
            "S->C",
            "Two mods, one channel, one registry entry.",
            Message::ModList(ModList {
                mods: vec![hello_lua(), landclaim()],
                channels: vec![ChannelInfo { name: "landclaim:sync".into(), id: 1 }],
                registry: BTreeMap::from([("blocks".into(), BTreeMap::from([("landclaim:marker".into(), 200)]))]),
            }),
        ),
        (
            "03-mod-list-registry",
            "S->C",
            "One content mod: the world's id map for blocks, items and cosmetics (cosmetic ids start at 1).",
            Message::ModList(ModList {
                mods: vec![hello_lua()],
                channels: vec![],
                registry: BTreeMap::from([
                    ("blocks".into(), BTreeMap::from([("hats:marble".into(), 1)])),
                    ("cosmetics".into(), BTreeMap::from([("hats:crown".into(), 1), ("hats:ember-hair".into(), 2)])),
                    ("items".into(), BTreeMap::from([("hats:crown-token".into(), 1107)])),
                ]),
            }),
        ),
        ("03-mod-list-empty", "S->C", "No mods, channels or registry.", Message::ModList(ModList::default())),
        (
            "04-file-request",
            "C->S",
            "Start downloading landclaim from the beginning.",
            Message::FileRequest(FileRequest { sha256: h("landclaim-1.4.0.bhmod"), offset: 0 }),
        ),
        (
            "04-file-request-resume",
            "C->S",
            "Resume / acknowledge after 3 chunks (98304 bytes).",
            Message::FileRequest(FileRequest { sha256: h("landclaim-1.4.0.bhmod"), offset: 3 * CHUNK_SIZE as u64 }),
        ),
        (
            "05-file-chunk-first",
            "S->C",
            "First full 32 KiB chunk of a 48213-byte file; data[i] = i % 251.",
            Message::FileChunk(FileChunk { sha256: h("landclaim-1.4.0.bhmod"), offset: 0, total: 48213, data: chunk0 }),
        ),
        (
            "05-file-chunk-last",
            "S->C",
            "Last chunk (15445 bytes at offset 32768); data[i] = (32768 + i) % 251.",
            Message::FileChunk(FileChunk {
                sha256: h("landclaim-1.4.0.bhmod"),
                offset: CHUNK_SIZE as u64,
                total: 48213,
                data: (CHUNK_SIZE..48213).map(|i| (i % 251) as u8).collect(),
            }),
        ),
        (
            "06-file-error",
            "S->C",
            "Unknown hash (code 1 = NOT_FOUND).",
            Message::FileError(FileError { sha256: h("nope"), code: file_error::NOT_FOUND }),
        ),
        (
            "07-ready",
            "C->S",
            "Player accepted; both mods loaded.",
            Message::Ready(Ready {
                accepted_hash: landclaim_hash,
                loaded: vec![("hello-lua".into(), "0.1.0".into()), ("landclaim".into(), "1.4.0".into())],
            }),
        ),
        (
            "08-reject-text",
            "S->C",
            "Proto mismatch text, followed by 0x26 03.",
            Message::RejectText(RejectText {
                reason_code: 3,
                text: "This server needs mod loader protocol 1. Update your mod loader with Patcher.".into(),
                url: Some("https://blockheads.club/docs/mods/players".into()),
            }),
        ),
        (
            "08-reject-text-no-url",
            "S->C",
            "Stale acceptedHash, no link.",
            Message::RejectText(RejectText {
                reason_code: 3,
                text: "The server's mods changed. Join again.".into(),
                url: None,
            }),
        ),
        (
            "10-channel",
            "both",
            "Channel 1, 5-byte payload.",
            Message::Channel(Channel { channel_id: 1, payload: b"hello".to_vec() }),
        ),
        (
            "10-channel-empty",
            "both",
            "Channel 0x1234, empty payload.",
            Message::Channel(Channel { channel_id: 0x1234, payload: vec![] }),
        ),
        (
            "11-channel-register",
            "both",
            "Late registration of channel 2.",
            Message::ChannelRegister(ChannelRegister { name: "landclaim:claims".into(), id: 2 }),
        ),
        (
            "12-cosmetics-full",
            "S->C",
            "Everything players wear, sent once after approval: two players, one with a hat and hair.",
            Message::Cosmetics(Cosmetics {
                full: true,
                players: BTreeMap::from([
                    ("player-a".into(), BTreeMap::from([("hair".into(), 2), ("hat".into(), 1)])),
                    ("player-b".into(), BTreeMap::from([("hat".into(), 1)])),
                ]),
            }),
        ),
        (
            "12-cosmetics-delta",
            "S->C",
            "A delta: player-a took the hat off (empty set), player-c puts a hat on.",
            Message::Cosmetics(Cosmetics {
                full: false,
                players: BTreeMap::from([
                    ("player-a".into(), BTreeMap::new()),
                    ("player-c".into(), BTreeMap::from([("hat".into(), 1)])),
                ]),
            }),
        ),
        (
            "13-service-call",
            "both",
            "Client mod shop-ui calls economy ^1.1 `balance(\"player-a\")`; the args are the canonical CBOR array.",
            Message::ServiceCall(ServiceCall {
                id: 7,
                mod_id: "shop-ui".into(),
                service: "economy".into(),
                version_req: "^1.1".into(),
                method: "balance".into(),
                args: cbor::encode(&Value::Array(vec![Value::text("player-a")])),
            }),
        ),
        (
            "13-service-call-no-args",
            "both",
            "A call without arguments (empty CBOR array).",
            Message::ServiceCall(ServiceCall {
                id: 4_000_000_000,
                mod_id: "landclaim".into(),
                service: "claims".into(),
                version_req: "*".into(),
                method: "count".into(),
                args: cbor::encode(&Value::Array(vec![])),
            }),
        ),
        (
            "14-service-result-ok",
            "both",
            "The answer to call 7: the integer 250.",
            Message::ServiceResult(ServiceResult { id: 7, outcome: Ok(cbor::encode(&Value::Uint(250))) }),
        ),
        (
            "14-service-result-error",
            "both",
            "The service is not exposed to the caller's side (NOT_PERMITTED).",
            Message::ServiceResult(ServiceResult {
                id: 7,
                outcome: Err(ServiceFailure {
                    code: service_error::NOT_PERMITTED,
                    message: "economy is not callable from clients".into(),
                }),
            }),
        ),
        (
            "14-service-result-provider-error",
            "both",
            "The provider returned an error of its own (PROVIDER_ERROR).",
            Message::ServiceResult(ServiceResult {
                id: 4_000_000_000,
                outcome: Err(ServiceFailure { code: service_error::PROVIDER_ERROR, message: "no such account".into() }),
            }),
        ),
        (
            "15-voice-key-client",
            "C->S",
            "The client's X25519 public key (bytes 0x01..=0x20).",
            Message::VoiceKey(VoiceKey { public: std::array::from_fn(|i| i as u8 + 1), peer: None, udp_port: None }),
        ),
        (
            "15-voice-key-server",
            "S->C",
            "The server's public key, the client's speaker id 7 and the voice UDP port.",
            Message::VoiceKey(VoiceKey {
                public: std::array::from_fn(|i| 0xa0 + i as u8),
                peer: Some(7),
                udp_port: Some(15152),
            }),
        ),
        (
            "16-voice-frame",
            "both",
            "Nonce 1 and a 19-byte sealed body (3-byte plaintext + 16-byte tag; content arbitrary here, see bhmod-voice for real frames).",
            Message::VoiceFrame(VoiceFrame { nonce: 1, sealed: (0..19).collect() }),
        ),
        (
            "17-voice-state-full",
            "S->C",
            "Full state: player-a talks, player-b has no mod loader; icon visibility.",
            Message::VoiceState(VoiceState {
                full: true,
                players: BTreeMap::from([("player-a".into(), "talking".into()), ("player-b".into(), "none".into())]),
                speakers: BTreeMap::from([("player-a".into(), 1)]),
                icons: BTreeMap::from([("no_voice".into(), true), ("speaking".into(), true)]),
            }),
        ),
        (
            "17-voice-state-delta",
            "S->C",
            "Delta: player-a stopped talking.",
            Message::VoiceState(VoiceState {
                full: false,
                players: BTreeMap::from([("player-a".into(), "ready".into())]),
                ..Default::default()
            }),
        ),
        (
            "18-voice-ctrl",
            "C->S",
            "The player muted the microphone but still hears others.",
            Message::VoiceCtrl(VoiceCtrl { self_mute: true, deafen: false }),
        ),
        (
            "19-settings-schema",
            "S->C",
            "One int field `radius` (8..=64, default 32); schema and values are canonical CBOR byte strings.",
            Message::SettingsSchema(SettingsSchema {
                mod_id: "voice".into(),
                schema: cbor::encode(&Value::Map(vec![(
                    Value::text("fields"),
                    Value::Array(vec![Value::Map(vec![
                        (Value::text("key"), Value::text("radius")),
                        (Value::text("type"), Value::text("int")),
                        (Value::text("min"), Value::Uint(8)),
                        (Value::text("max"), Value::Uint(64)),
                        (Value::text("default"), Value::Uint(32)),
                    ])]),
                )])),
                values: cbor::encode(&Value::Map(vec![(Value::text("radius"), Value::Uint(32))])),
            }),
        ),
        (
            "1a-settings-set",
            "C->S",
            "An admin sets voice.radius to 48.",
            Message::SettingsSet(SettingsSet {
                mod_id: "voice".into(),
                key: "radius".into(),
                value: cbor::encode(&Value::Uint(48)),
            }),
        ),
        ("1b-settings-result-ok", "S->C", "Stored.", Message::SettingsResult(SettingsResult { outcome: Ok(()) })),
        (
            "1b-settings-result-error",
            "S->C",
            "Out of range (INVALID_VALUE).",
            Message::SettingsResult(SettingsResult {
                outcome: Err(SettingsFailure {
                    code: settings_error::INVALID_VALUE,
                    message: Some("radius must be 8..=64".into()),
                }),
            }),
        ),
        (
            "1b-settings-result-denied",
            "S->C",
            "Not an admin (NOT_PERMITTED), no message.",
            Message::SettingsResult(SettingsResult {
                outcome: Err(SettingsFailure { code: settings_error::NOT_PERMITTED, message: None }),
            }),
        ),
    ]
}

/// (file stem, hex, expected error code)
fn invalid() -> Vec<(&'static str, String, &'static str)> {
    let hash = to_hex(&h("x"));
    let cbor_hex = |v: Value| to_hex(&cbor::encode(&v));
    let call_body = |id: Value, args: Value| {
        cbor_hex(Value::Map(vec![
            (Value::text("id"), id),
            (Value::text("mod"), Value::text("a")),
            (Value::text("service"), Value::text("s")),
            (Value::text("req"), Value::text("*")),
            (Value::text("method"), Value::text("m")),
            (Value::text("args"), args),
        ]))
    };
    let req = |offset: Value| {
        cbor_hex(Value::Map(vec![
            (Value::text("sha256"), Value::Bytes(h("x").to_vec())),
            (Value::text("offset"), offset),
        ]))
    };
    vec![
        ("not-0xbc", "2603".into(), "not-mod-packet"),
        ("no-opcode", "bc".into(), "truncated"),
        ("unknown-opcode", "bc09a0".into(), "unknown-op"),
        ("hello-not-map", "bc0080".into(), "not-map"),
        ("hello-indefinite-map", "bc00bfff".into(), "cbor"),
        ("hello-unsorted-keys", "bc00a2616200616100".into(), "cbor"),
        (
            "file-request-non-shortest-int",
            format!("bc04{}", req(Value::Uint(0)).replacen("666f666673657400", "666f66667365741800", 1)),
            "cbor",
        ),
        ("list-request-trailing", "bc02a000".into(), "cbor"),
        ("file-request-missing-offset", format!("bc04a1667368613235365820{hash}"), "missing"),
        (
            "file-request-short-hash",
            format!(
                "bc04{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("sha256"), Value::Bytes(vec![1, 2])),
                    (Value::text("offset"), Value::Uint(0))
                ]))
            ),
            "type",
        ),
        ("file-request-text-offset", format!("bc04{}", req(Value::text("0"))), "type"),
        ("chunk-short-header", format!("bc05{hash}"), "truncated"),
        ("chunk-empty-data", format!("bc05{hash}{}{}", "00".repeat(8), "0100000000000000"), "invalid"),
        ("chunk-unaligned", format!("bc05{hash}{}{}ff", "0100000000000000", "0200000000000000"), "invalid"),
        ("chunk-past-end", format!("bc05{hash}{}{}ffff", "00".repeat(8), "0100000000000000"), "invalid"),
        ("channel-no-id", "bc1001".into(), "truncated"),
        (
            "channel-register-id-too-big",
            format!(
                "bc11{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("name"), Value::text("a:b")),
                    (Value::text("id"), Value::Uint(70000))
                ]))
            ),
            "type",
        ),
        (
            "cosmetics-id-text",
            format!(
                "bc12{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("full"), Value::Bool(true)),
                    (
                        Value::text("players"),
                        Value::Map(vec![(Value::text("p"), Value::Map(vec![(Value::text("hat"), Value::text("1"))]))])
                    )
                ]))
            ),
            "type",
        ),
        (
            "cosmetics-missing-full",
            format!("bc12{}", cbor_hex(Value::Map(vec![(Value::text("players"), Value::Map(vec![]))]))),
            "missing",
        ),
        (
            "service-call-missing-method",
            format!(
                "bc13{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("id"), Value::Uint(1)),
                    (Value::text("mod"), Value::text("a")),
                    (Value::text("service"), Value::text("s")),
                    (Value::text("req"), Value::text("*")),
                    (Value::text("args"), Value::Bytes(vec![0x80])),
                ]))
            ),
            "missing",
        ),
        (
            "service-call-id-too-big",
            format!("bc13{}", call_body(Value::Uint(1 << 32), Value::Bytes(vec![0x80]))),
            "type",
        ),
        ("service-call-args-text", format!("bc13{}", call_body(Value::Uint(1), Value::text("80"))), "type"),
        (
            "service-result-ok-without-value",
            format!(
                "bc14{}",
                cbor_hex(Value::Map(vec![(Value::text("id"), Value::Uint(1)), (Value::text("ok"), Value::Bool(true))]))
            ),
            "missing",
        ),
        (
            "service-result-big-code",
            format!(
                "bc14{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("id"), Value::Uint(1)),
                    (Value::text("ok"), Value::Bool(false)),
                    (Value::text("code"), Value::Uint(256)),
                    (Value::text("message"), Value::text("x")),
                ]))
            ),
            "type",
        ),
        (
            "server-hello-players-without-max",
            format!(
                "bc01{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("count"), Value::Uint(0)),
                    (Value::text("proto"), Value::Uint(1)),
                    (Value::text("players"), Value::Uint(3)),
                    (Value::text("modsHash"), Value::Bytes(h("x").to_vec())),
                    (Value::text("clientRequired"), Value::Bool(false)),
                ]))
            ),
            "missing",
        ),
        (
            "server-hello-players-too-big",
            format!(
                "bc01{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("count"), Value::Uint(0)),
                    (Value::text("proto"), Value::Uint(1)),
                    (Value::text("players"), Value::Uint(1 << 32)),
                    (Value::text("modsHash"), Value::Bytes(h("x").to_vec())),
                    (Value::text("maxPlayers"), Value::Uint(1)),
                    (Value::text("clientRequired"), Value::Bool(false)),
                ]))
            ),
            "type",
        ),
        ("voice-frame-short-nonce", "bc1601020304".into(), "truncated"),
        ("voice-frame-no-tag", format!("bc16{}{}", "01".repeat(8), "00".repeat(15)), "invalid"),
        (
            "voice-key-short",
            format!("bc15{}", cbor_hex(Value::Map(vec![(Value::text("pub"), Value::Bytes(vec![1; 31]))]))),
            "type",
        ),
        (
            "voice-key-port-too-big",
            format!(
                "bc15{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("pub"), Value::Bytes(vec![1; 32])),
                    (Value::text("udpPort"), Value::Uint(65536)),
                ]))
            ),
            "type",
        ),
        (
            "voice-state-icon-not-bool",
            format!(
                "bc17{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("full"), Value::Bool(true)),
                    (Value::text("players"), Value::Map(vec![])),
                    (Value::text("icons"), Value::Map(vec![(Value::text("speaking"), Value::Uint(1))])),
                ]))
            ),
            "type",
        ),
        (
            "voice-ctrl-missing-deafen",
            format!("bc18{}", cbor_hex(Value::Map(vec![(Value::text("selfMute"), Value::Bool(true))]))),
            "missing",
        ),
        (
            "settings-set-value-text",
            format!(
                "bc1a{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("mod"), Value::text("voice")),
                    (Value::text("key"), Value::text("radius")),
                    (Value::text("value"), Value::text("48")),
                ]))
            ),
            "type",
        ),
        (
            "settings-result-error-without-code",
            format!("bc1b{}", cbor_hex(Value::Map(vec![(Value::text("ok"), Value::Bool(false))]))),
            "missing",
        ),
        (
            "server-hello-features-not-text",
            format!(
                "bc01{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("count"), Value::Uint(0)),
                    (Value::text("proto"), Value::Uint(1)),
                    (Value::text("features"), Value::Array(vec![Value::Uint(1)])),
                    (Value::text("modsHash"), Value::Bytes(h("x").to_vec())),
                    (Value::text("clientRequired"), Value::Bool(false)),
                ]))
            ),
            "type",
        ),
        (
            "reject-text-big-reason",
            format!(
                "bc08{}",
                cbor_hex(Value::Map(vec![
                    (Value::text("reasonCode"), Value::Uint(256)),
                    (Value::text("text"), Value::text("x"))
                ]))
            ),
            "type",
        ),
    ]
}

fn code(e: &Error) -> &'static str {
    match e {
        Error::NotModPacket => "not-mod-packet",
        Error::Truncated => "truncated",
        Error::UnknownOp(_) => "unknown-op",
        Error::Cbor(_) => "cbor",
        Error::Missing(_) => "missing",
        Error::Type(_) => "type",
        Error::TooLarge(_) => "too-large",
        Error::NotMap => "not-map",
        Error::Invalid(_) => "invalid",
    }
}

fn check_file(path: PathBuf, expected: &str) {
    if bless() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, expected).unwrap();
    }
    let on_disk = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(on_disk, expected, "{} differs (run with BHMOD_BLESS=1 after an intended change)", path.display());
}

#[test]
fn valid_vectors() {
    let mut index = Vec::new();
    for (stem, dir_, desc, msg) in valid() {
        let bytes = msg.encode();
        let hex = to_hex(&bytes);
        check_file(dir().join(format!("{stem}.hex")), &format!("{hex}\n"));
        let from_disk = from_hex(&fs::read_to_string(dir().join(format!("{stem}.hex"))).unwrap());
        assert_eq!(Message::decode(&from_disk).unwrap(), msg, "{stem}");
        let fields = match &msg {
            Message::FileChunk(c) => json!({
                "raw": true, "sha256": to_hex(&c.sha256), "offset": c.offset, "total": c.total,
                "dataLength": c.data.len(), "dataSha256": sha_hex(&c.data),
            }),
            Message::Channel(c) => json!({ "raw": true, "channelId": c.channel_id, "payload": to_hex(&c.payload) }),
            Message::VoiceFrame(f) => json!({ "raw": true, "nonce": f.nonce, "sealed": to_hex(&f.sealed) }),
            _ => cbor_json(&cbor::decode(&bytes[2..]).unwrap()),
        };
        index.push(json!({
            "file": format!("{stem}.hex"), "op": format!("0x{:02x}", bytes[1]), "direction": dir_,
            "description": desc, "length": bytes.len(), "body": fields,
        }));
    }
    let mut invalid_index = Vec::new();
    for (stem, hex, expected) in invalid() {
        check_file(dir().join("invalid").join(format!("{stem}.hex")), &format!("{hex}\n"));
        let err = Message::decode(&from_hex(&hex)).unwrap_err();
        assert_eq!(code(&err), expected, "{stem}: {err}");
        invalid_index.push(json!({ "file": format!("invalid/{stem}.hex"), "error": expected }));
    }
    let doc = json!({
        "proto": PROTO,
        "note": "Generated by crates/bhmod-proto/tests/vectors.rs. Each .hex file is one full ENet packet payload (0xBC, sub-opcode, body) as lowercase hex plus a newline. CBOR byte strings appear in `body` as {\"$bytes\": hex}.",
        "valid": index,
        "invalid": invalid_index,
        "modsHash": mods_hash_vectors(),
        "vanillaReject": { "packet": to_hex(&vanilla_reject_packet()), "meaning": "0x26 rejection, reason 3 CLIENT_TOO_OLD" },
    });
    check_file(dir().join("vectors.json"), &format!("{}\n", serde_json::to_string_pretty(&doc).unwrap()));
}

fn sha_hex(data: &[u8]) -> String {
    use sha2::Digest;
    to_hex(&sha2::Sha256::digest(data))
}

fn mods_hash_vectors() -> Json {
    let a = h("landclaim-1.4.0.bhmod");
    let b = h("economy-1.1.0.bhmod");
    type Case<'a> = (Vec<(&'a str, &'a str, &'a Hash)>, &'a str);
    let cases: Vec<Case> = vec![
        (vec![], "no mods"),
        (vec![("landclaim", "1.4.0", &a)], "one mod"),
        (vec![("landclaim", "1.4.0", &a), ("economy", "1.1.0", &b)], "two mods, given unsorted"),
    ];
    Json::Array(
        cases
            .into_iter()
            .map(|(mods, desc)| {
                let lines: Vec<Json> =
                    mods.iter().map(|(id, v, h)| json!({ "id": id, "version": v, "sha256": to_hex(*h) })).collect();
                json!({ "description": desc, "mods": lines, "modsHash": to_hex(&mods_hash(mods.iter().copied())) })
            })
            .collect(),
    )
}

#[test]
fn mods_hash_order_independent() {
    let a = h("a");
    let b = h("b");
    assert_eq!(
        mods_hash([("x", "1.0.0", &a), ("y", "1.0.0", &b)]),
        mods_hash([("y", "1.0.0", &b), ("x", "1.0.0", &a)])
    );
    // sha256("") for the empty set.
    assert_eq!(hex(&mods_hash([])), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
}
