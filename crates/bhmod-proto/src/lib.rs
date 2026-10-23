//! Encode and decode the Blockheads Club **0xBC** mod protocol (spec: `docs/protocol-0xbc.md`).
//!
//! A packet is one ENet packet: `0xBC`, a sub-opcode byte, then the body. Most bodies are canonical CBOR maps
//! ([`cbor`]); `FILE_CHUNK` and `CHANNEL` carry raw bytes behind a small fixed header.
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest, Sha256};

pub mod cbor;

use cbor::Value;

/// The packet type byte ("Blockheads Club").
pub const PACKET_TYPE: u8 = 0xBC;
/// The protocol version carried in `HELLO` / `SERVER_HELLO` / `bhcModLoader`.
pub const PROTO: u64 = 1;
/// File chunk size: 32 KiB. Every chunk but the last of a file is exactly this long.
pub const CHUNK_SIZE: usize = 32 * 1024;
/// Most unacknowledged chunks in flight per peer.
pub const MAX_IN_FLIGHT: u64 = 8;
/// Largest `CHANNEL` payload.
pub const MAX_CHANNEL_PAYLOAD: usize = 32 * 1024;
/// Largest CBOR body.
pub const MAX_BODY: usize = 1024 * 1024;
/// How long a modded client waits for `SERVER_HELLO` before it carries on as a vanilla client.
pub const HELLO_TIMEOUT_MS: u64 = 1500;
/// The key a modded client adds to its 0x1f player-information plist.
pub const PLIST_KEY: &str = "bhcModLoader";
/// The vanilla rejection reason (`0x26 <reason>`) used for clients without the mod loader: CLIENT_TOO_OLD.
pub const REJECT_CLIENT_TOO_OLD: u8 = 3;

/// `FILE_ERROR` codes.
pub mod file_error {
    /// The server has no file with that hash.
    pub const NOT_FOUND: u64 = 1;
    /// The offset is past the end of the file or not a multiple of the chunk size.
    pub const BAD_OFFSET: u64 = 2;
    /// Rate limited or too many transfers; retry after a short delay.
    pub const BUSY: u64 = 3;
    /// The server failed to read the file.
    pub const INTERNAL: u64 = 4;
}

/// A SHA-256 digest.
pub type Hash = [u8; 32];

/// Decoding errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The packet does not start with [`PACKET_TYPE`].
    NotModPacket,
    /// The packet ends before the sub-opcode or a fixed header field.
    Truncated,
    /// An unknown sub-opcode; receivers ignore such packets.
    UnknownOp(u8),
    /// The body is not valid canonical CBOR of the supported subset.
    Cbor(&'static str),
    /// A required key is missing.
    Missing(&'static str),
    /// A key has the wrong type or is out of range.
    Type(&'static str),
    /// A size limit was exceeded.
    TooLarge(&'static str),
    /// The body is not a CBOR map.
    NotMap,
    /// Inconsistent fields.
    Invalid(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotModPacket => write!(f, "not a 0xBC packet"),
            Error::Truncated => write!(f, "truncated packet"),
            Error::UnknownOp(op) => write!(f, "unknown sub-opcode 0x{op:02x}"),
            Error::Cbor(why) => write!(f, "bad CBOR: {why}"),
            Error::Missing(k) => write!(f, "missing key `{k}`"),
            Error::Type(k) => write!(f, "wrong type or range for `{k}`"),
            Error::TooLarge(what) => write!(f, "{what} too large"),
            Error::NotMap => write!(f, "body is not a CBOR map"),
            Error::Invalid(why) => write!(f, "invalid message: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Sub-opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    /// C->S, first message of a modded client.
    Hello = 0x00,
    /// S->C, answer to `HELLO`.
    ServerHello = 0x01,
    /// C->S, asks for the mod list.
    ListRequest = 0x02,
    /// S->C, the mod list.
    ModList = 0x03,
    /// C->S, asks for (the rest of) a file; also acknowledges chunks.
    FileRequest = 0x04,
    /// S->C, one chunk of a file.
    FileChunk = 0x05,
    /// S->C, a file request failed.
    FileError = 0x06,
    /// C->S, the client accepted and loaded the mods.
    Ready = 0x07,
    /// S->C, readable rejection text for modded clients (followed by 0x26).
    RejectText = 0x08,
    /// Both directions, a mod channel payload.
    Channel = 0x10,
    /// Both directions, late channel registration.
    ChannelRegister = 0x11,
    /// S->C, the cosmetics the players wear (M8).
    Cosmetics = 0x12,
}

impl Op {
    /// Maps a byte to an opcode.
    pub fn from_u8(b: u8) -> Option<Op> {
        Some(match b {
            0x00 => Op::Hello,
            0x01 => Op::ServerHello,
            0x02 => Op::ListRequest,
            0x03 => Op::ModList,
            0x04 => Op::FileRequest,
            0x05 => Op::FileChunk,
            0x06 => Op::FileError,
            0x07 => Op::Ready,
            0x08 => Op::RejectText,
            0x10 => Op::Channel,
            0x11 => Op::ChannelRegister,
            0x12 => Op::Cosmetics,
            _ => return None,
        })
    }
}

/// `HELLO` (C->S).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// Protocol version ([`PROTO`]).
    pub proto: u64,
    /// Mod loader version (semver text).
    pub loader: String,
    /// Client platform, e.g. `android-armv7`, `pc-linux-x86_64`.
    pub platform: String,
    /// Highest `blockheads:api` version the loader implements.
    pub api: String,
    /// Runtimes the loader has: `wasm`, `lua`, `jvm`, `native`.
    pub runtimes: Vec<String>,
    /// SHA-256 of every package already in the client cache (may be empty).
    pub cached: Vec<Hash>,
}

/// `SERVER_HELLO` (S->C).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerHello {
    /// Protocol version ([`PROTO`]).
    pub proto: u64,
    /// [`mods_hash`] of the mods the client must run.
    pub mods_hash: Hash,
    /// True if a client without these mods is rejected.
    pub client_required: bool,
    /// Number of entries in the coming `MOD_LIST`.
    pub count: u64,
}

/// Permissions of a listed mod, as shown in the accept dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModPermissions {
    /// `none`, `read` or `write`.
    pub world: String,
    /// Player permissions (`chat`, `kick`, ...).
    pub players: Vec<String>,
    /// Outbound HTTP hosts.
    pub net_http: Vec<String>,
    /// May open channels.
    pub channels: bool,
    /// Native / unsandboxed code.
    pub native: bool,
}

/// One entry of `MOD_LIST`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInfo {
    /// Mod id.
    pub id: String,
    /// Mod version.
    pub version: String,
    /// Display name per language.
    pub name: BTreeMap<String, String>,
    /// Description per language.
    pub description: BTreeMap<String, String>,
    /// `server`, `client`.
    pub sides: Vec<String>,
    /// Whether the client must run it to join.
    pub client_required: bool,
    /// SHA-256 of the `.bhmod`.
    pub sha256: Hash,
    /// Size of the `.bhmod` in bytes.
    pub size: u64,
    /// Requested permissions.
    pub permissions: ModPermissions,
    /// Optional download URL (the index URL for index-listed mods); the hash must match either way.
    pub source_url: Option<String>,
    /// True if the public index lists this id, version and hash.
    pub index_listed: bool,
}

/// A named channel and its numeric id for `CHANNEL` packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    /// `<mod-id>:<name>`.
    pub name: String,
    /// Id used on the wire.
    pub id: u16,
}

/// `MOD_LIST` (S->C).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModList {
    /// Mods the client should (or may) run, sorted by id.
    pub mods: Vec<ModInfo>,
    /// Channels open at this time.
    pub channels: Vec<ChannelInfo>,
    /// Content id map: kind (`blocks`, `items`, `cosmetics`, ...) -> content key -> numeric id.
    pub registry: BTreeMap<String, BTreeMap<String, u64>>,
}

/// `FILE_REQUEST` (C->S).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRequest {
    /// The package wanted.
    pub sha256: Hash,
    /// Bytes the client already has (a multiple of [`CHUNK_SIZE`], or the file size as final acknowledgement).
    pub offset: u64,
}

/// `FILE_CHUNK` (S->C). Wire layout: `sha256[32] offset:u64le total:u64le data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChunk {
    /// The file.
    pub sha256: Hash,
    /// Where `data` starts.
    pub offset: u64,
    /// File size.
    pub total: u64,
    /// 1..=[`CHUNK_SIZE`] bytes.
    pub data: Vec<u8>,
}

/// `FILE_ERROR` (S->C).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    /// The file.
    pub sha256: Hash,
    /// One of [`file_error`].
    pub code: u64,
}

/// `READY` (C->S).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    /// The `modsHash` from `SERVER_HELLO` the player accepted.
    pub accepted_hash: Hash,
    /// `(id, version)` of every mod the client loaded.
    pub loaded: Vec<(String, String)>,
}

/// `REJECT_TEXT` (S->C).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectText {
    /// The vanilla 0x26 reason that follows.
    pub reason_code: u8,
    /// Human readable text (English or the client's language).
    pub text: String,
    /// Optional help link.
    pub url: Option<String>,
}

/// `CHANNEL` (both). Wire layout: `channel_id:u16le payload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// Id from `MOD_LIST.channels` or `CHANNEL_REGISTER`.
    pub channel_id: u16,
    /// Up to [`MAX_CHANNEL_PAYLOAD`] bytes.
    pub payload: Vec<u8>,
}

/// `CHANNEL_REGISTER` (both).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRegister {
    /// `<mod-id>:<name>`.
    pub name: String,
    /// Id used on the wire.
    pub id: u16,
}

/// `COSMETICS` (S->C): the cosmetics players wear, by the numeric ids of the `cosmetics` registry.
///
/// `full = true` replaces everything the client knew (sent once after approval); otherwise it is a delta: every
/// listed player's set is replaced, and a player with an empty set wears nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cosmetics {
    /// True: replace the whole table.
    pub full: bool,
    /// Vanilla `playerID` -> slot (`hat`, `hair`, ...) -> cosmetic id.
    pub players: BTreeMap<String, BTreeMap<String, u64>>,
}

/// A decoded 0xBC packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// 0x00.
    Hello(Hello),
    /// 0x01.
    ServerHello(ServerHello),
    /// 0x02.
    ListRequest,
    /// 0x03.
    ModList(ModList),
    /// 0x04.
    FileRequest(FileRequest),
    /// 0x05.
    FileChunk(FileChunk),
    /// 0x06.
    FileError(FileError),
    /// 0x07.
    Ready(Ready),
    /// 0x08.
    RejectText(RejectText),
    /// 0x10.
    Channel(Channel),
    /// 0x11.
    ChannelRegister(ChannelRegister),
    /// 0x12.
    Cosmetics(Cosmetics),
}

fn text(s: &str) -> Value {
    Value::text(s)
}

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(entries.into_iter().map(|(k, v)| (text(k), v)).collect())
}

fn texts(items: &[String]) -> Value {
    Value::Array(items.iter().map(|s| text(s)).collect())
}

fn localized(m: &BTreeMap<String, String>) -> Value {
    Value::Map(m.iter().map(|(k, v)| (text(k), text(v))).collect())
}

fn hash(h: &Hash) -> Value {
    Value::Bytes(h.to_vec())
}

impl Message {
    /// The sub-opcode.
    pub fn op(&self) -> Op {
        match self {
            Message::Hello(_) => Op::Hello,
            Message::ServerHello(_) => Op::ServerHello,
            Message::ListRequest => Op::ListRequest,
            Message::ModList(_) => Op::ModList,
            Message::FileRequest(_) => Op::FileRequest,
            Message::FileChunk(_) => Op::FileChunk,
            Message::FileError(_) => Op::FileError,
            Message::Ready(_) => Op::Ready,
            Message::RejectText(_) => Op::RejectText,
            Message::Channel(_) => Op::Channel,
            Message::ChannelRegister(_) => Op::ChannelRegister,
            Message::Cosmetics(_) => Op::Cosmetics,
        }
    }

    /// Encodes the full packet, starting with `0xBC` and the sub-opcode.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![PACKET_TYPE, self.op() as u8];
        match self.body() {
            Ok(v) => out.extend(cbor::encode(&v)),
            Err(raw) => out.extend(raw),
        }
        out
    }

    /// The CBOR body, or `Err(raw bytes)` for the two raw messages.
    fn body(&self) -> Result<Value, Vec<u8>> {
        Ok(match self {
            Message::Hello(h) => map(vec![
                ("proto", Value::Uint(h.proto)),
                ("loader", text(&h.loader)),
                ("platform", text(&h.platform)),
                ("api", text(&h.api)),
                ("runtimes", texts(&h.runtimes)),
                ("cached", Value::Array(h.cached.iter().map(hash).collect())),
            ]),
            Message::ServerHello(s) => map(vec![
                ("proto", Value::Uint(s.proto)),
                ("modsHash", hash(&s.mods_hash)),
                ("clientRequired", Value::Bool(s.client_required)),
                ("count", Value::Uint(s.count)),
            ]),
            Message::ListRequest => map(vec![]),
            Message::ModList(l) => map(vec![
                ("mods", Value::Array(l.mods.iter().map(mod_info).collect())),
                ("channels", Value::Array(l.channels.iter().map(|c| channel_entry(&c.name, c.id)).collect())),
                (
                    "registry",
                    Value::Map(
                        l.registry
                            .iter()
                            .map(|(kind, ids)| {
                                (
                                    text(kind),
                                    Value::Map(ids.iter().map(|(k, id)| (text(k), Value::Uint(*id))).collect()),
                                )
                            })
                            .collect(),
                    ),
                ),
            ]),
            Message::FileRequest(r) => map(vec![("sha256", hash(&r.sha256)), ("offset", Value::Uint(r.offset))]),
            Message::FileError(e) => map(vec![("sha256", hash(&e.sha256)), ("code", Value::Uint(e.code))]),
            Message::Ready(r) => map(vec![
                ("acceptedHash", hash(&r.accepted_hash)),
                (
                    "loaded",
                    Value::Array(
                        r.loaded.iter().map(|(id, v)| map(vec![("id", text(id)), ("version", text(v))])).collect(),
                    ),
                ),
            ]),
            Message::RejectText(r) => {
                let mut m = vec![("reasonCode", Value::Uint(u64::from(r.reason_code))), ("text", text(&r.text))];
                if let Some(u) = &r.url {
                    m.push(("url", text(u)));
                }
                map(m)
            }
            Message::ChannelRegister(c) => channel_entry(&c.name, c.id),
            Message::Cosmetics(c) => map(vec![
                ("full", Value::Bool(c.full)),
                (
                    "players",
                    Value::Map(
                        c.players
                            .iter()
                            .map(|(player, slots)| {
                                (
                                    text(player),
                                    Value::Map(slots.iter().map(|(s, id)| (text(s), Value::Uint(*id))).collect()),
                                )
                            })
                            .collect(),
                    ),
                ),
            ]),
            Message::FileChunk(c) => {
                let mut raw = c.sha256.to_vec();
                raw.extend(c.offset.to_le_bytes());
                raw.extend(c.total.to_le_bytes());
                raw.extend(&c.data);
                return Err(raw);
            }
            Message::Channel(c) => {
                let mut raw = c.channel_id.to_le_bytes().to_vec();
                raw.extend(&c.payload);
                return Err(raw);
            }
        })
    }

    /// Decodes a full packet (starting with `0xBC`).
    pub fn decode(packet: &[u8]) -> Result<Message, Error> {
        if packet.first() != Some(&PACKET_TYPE) {
            return Err(Error::NotModPacket);
        }
        let op_byte = *packet.get(1).ok_or(Error::Truncated)?;
        let op = Op::from_u8(op_byte).ok_or(Error::UnknownOp(op_byte))?;
        let body = &packet[2..];
        match op {
            Op::FileChunk => decode_chunk(body),
            Op::Channel => decode_channel(body),
            _ if body.len() > MAX_BODY => Err(Error::TooLarge("body")),
            _ => decode_cbor(op, &Fields::from(cbor::decode(body)?)?),
        }
    }
}

fn decode_cbor(op: Op, f: &Fields) -> Result<Message, Error> {
    Ok(match op {
        Op::Hello => Message::Hello(Hello {
            proto: f.uint("proto")?,
            loader: f.text("loader")?,
            platform: f.text("platform")?,
            api: f.text("api")?,
            runtimes: f.texts("runtimes")?,
            cached: f.array("cached")?.iter().map(|v| as_hash(v, "cached")).collect::<Result<_, _>>()?,
        }),
        Op::ServerHello => Message::ServerHello(ServerHello {
            proto: f.uint("proto")?,
            mods_hash: f.hash("modsHash")?,
            client_required: f.bool("clientRequired")?,
            count: f.uint("count")?,
        }),
        Op::ListRequest => Message::ListRequest,
        Op::ModList => Message::ModList(ModList {
            mods: f.array("mods")?.iter().map(|v| Fields::of(v, "mods")?.mod_info()).collect::<Result<_, _>>()?,
            channels: f
                .array("channels")?
                .iter()
                .map(|v| {
                    let c = Fields::of(v, "channels")?;
                    Ok(ChannelInfo { name: c.text("name")?, id: c.u16("id")? })
                })
                .collect::<Result<_, Error>>()?,
            registry: f
                .map("registry")?
                .0
                .iter()
                .map(|(kind, ids)| {
                    let ids = Fields::of(ids, "registry")?
                        .0
                        .iter()
                        .map(|(k, id)| Ok((as_text(k, "registry")?, as_uint(id, "registry")?)))
                        .collect::<Result<_, Error>>()?;
                    Ok((as_text(kind, "registry")?, ids))
                })
                .collect::<Result<_, Error>>()?,
        }),
        Op::FileRequest => Message::FileRequest(FileRequest { sha256: f.hash("sha256")?, offset: f.uint("offset")? }),
        Op::FileError => Message::FileError(FileError { sha256: f.hash("sha256")?, code: f.uint("code")? }),
        Op::Ready => Message::Ready(Ready {
            accepted_hash: f.hash("acceptedHash")?,
            loaded: f
                .array("loaded")?
                .iter()
                .map(|v| {
                    let l = Fields::of(v, "loaded")?;
                    Ok((l.text("id")?, l.text("version")?))
                })
                .collect::<Result<_, Error>>()?,
        }),
        Op::RejectText => Message::RejectText(RejectText {
            reason_code: u8::try_from(f.uint("reasonCode")?).map_err(|_| Error::Type("reasonCode"))?,
            text: f.text("text")?,
            url: f.opt_text("url")?,
        }),
        Op::Cosmetics => Message::Cosmetics(Cosmetics {
            full: f.bool("full")?,
            players: f
                .map("players")?
                .0
                .iter()
                .map(|(player, slots)| {
                    let slots = Fields::of(slots, "players")?
                        .0
                        .iter()
                        .map(|(s, id)| Ok((as_text(s, "players")?, as_uint(id, "players")?)))
                        .collect::<Result<_, Error>>()?;
                    Ok((as_text(player, "players")?, slots))
                })
                .collect::<Result<_, Error>>()?,
        }),
        // FILE_CHUNK and CHANNEL never reach here; CHANNEL_REGISTER is the remaining CBOR message.
        _ => Message::ChannelRegister(ChannelRegister { name: f.text("name")?, id: f.u16("id")? }),
    })
}

fn channel_entry(name: &str, id: u16) -> Value {
    map(vec![("name", text(name)), ("id", Value::Uint(u64::from(id)))])
}

fn mod_info(m: &ModInfo) -> Value {
    let p = &m.permissions;
    let mut e = vec![
        ("id", text(&m.id)),
        ("version", text(&m.version)),
        ("name", localized(&m.name)),
        ("description", localized(&m.description)),
        ("sides", texts(&m.sides)),
        ("clientRequired", Value::Bool(m.client_required)),
        ("sha256", hash(&m.sha256)),
        ("size", Value::Uint(m.size)),
        (
            "permissions",
            map(vec![
                ("world", text(&p.world)),
                ("players", texts(&p.players)),
                ("netHttp", texts(&p.net_http)),
                ("channels", Value::Bool(p.channels)),
                ("native", Value::Bool(p.native)),
            ]),
        ),
        ("indexListed", Value::Bool(m.index_listed)),
    ];
    if let Some(u) = &m.source_url {
        e.push(("sourceUrl", text(u)));
    }
    map(e)
}

fn decode_chunk(body: &[u8]) -> Result<Message, Error> {
    if body.len() < 48 {
        return Err(Error::Truncated);
    }
    let sha256: Hash = body[..32].try_into().expect("32 bytes");
    let offset = u64::from_le_bytes(body[32..40].try_into().expect("8 bytes"));
    let total = u64::from_le_bytes(body[40..48].try_into().expect("8 bytes"));
    let data = body[48..].to_vec();
    if data.len() > CHUNK_SIZE {
        return Err(Error::TooLarge("chunk"));
    }
    let end = offset.checked_add(data.len() as u64);
    if data.is_empty() || offset % CHUNK_SIZE as u64 != 0 || end.is_none_or(|e| e > total) {
        return Err(Error::Invalid("chunk must be non-empty, chunk-aligned and inside the file"));
    }
    Ok(Message::FileChunk(FileChunk { sha256, offset, total, data }))
}

fn decode_channel(body: &[u8]) -> Result<Message, Error> {
    if body.len() < 2 {
        return Err(Error::Truncated);
    }
    let payload = body[2..].to_vec();
    if payload.len() > MAX_CHANNEL_PAYLOAD {
        return Err(Error::TooLarge("channel payload"));
    }
    Ok(Message::Channel(Channel { channel_id: u16::from_le_bytes([body[0], body[1]]), payload }))
}

/// A CBOR map with typed accessors. Unknown keys are ignored.
struct Fields(Vec<(Value, Value)>);

impl Fields {
    fn from(v: Value) -> Result<Fields, Error> {
        match v {
            Value::Map(m) => Ok(Fields(m)),
            _ => Err(Error::NotMap),
        }
    }

    fn of(v: &Value, key: &'static str) -> Result<Fields, Error> {
        match v {
            Value::Map(m) => Ok(Fields(m.clone())),
            _ => Err(Error::Type(key)),
        }
    }

    fn get(&self, key: &'static str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| *k == Value::text(key)).map(|(_, v)| v)
    }

    fn req(&self, key: &'static str) -> Result<&Value, Error> {
        self.get(key).ok_or(Error::Missing(key))
    }

    fn uint(&self, key: &'static str) -> Result<u64, Error> {
        as_uint(self.req(key)?, key)
    }

    fn u16(&self, key: &'static str) -> Result<u16, Error> {
        u16::try_from(self.uint(key)?).map_err(|_| Error::Type(key))
    }

    fn text(&self, key: &'static str) -> Result<String, Error> {
        as_text(self.req(key)?, key)
    }

    fn opt_text(&self, key: &'static str) -> Result<Option<String>, Error> {
        self.get(key).map(|v| as_text(v, key)).transpose()
    }

    fn bool(&self, key: &'static str) -> Result<bool, Error> {
        match self.req(key)? {
            Value::Bool(b) => Ok(*b),
            _ => Err(Error::Type(key)),
        }
    }

    fn hash(&self, key: &'static str) -> Result<Hash, Error> {
        as_hash(self.req(key)?, key)
    }

    fn array(&self, key: &'static str) -> Result<&[Value], Error> {
        match self.req(key)? {
            Value::Array(a) => Ok(a),
            _ => Err(Error::Type(key)),
        }
    }

    fn texts(&self, key: &'static str) -> Result<Vec<String>, Error> {
        self.array(key)?.iter().map(|v| as_text(v, key)).collect()
    }

    fn map(&self, key: &'static str) -> Result<Fields, Error> {
        Fields::of(self.req(key)?, key)
    }

    fn localized(&self, key: &'static str) -> Result<BTreeMap<String, String>, Error> {
        self.map(key)?.0.iter().map(|(k, v)| Ok((as_text(k, key)?, as_text(v, key)?))).collect()
    }

    fn mod_info(&self) -> Result<ModInfo, Error> {
        let p = self.map("permissions")?;
        Ok(ModInfo {
            id: self.text("id")?,
            version: self.text("version")?,
            name: self.localized("name")?,
            description: self.localized("description")?,
            sides: self.texts("sides")?,
            client_required: self.bool("clientRequired")?,
            sha256: self.hash("sha256")?,
            size: self.uint("size")?,
            permissions: ModPermissions {
                world: p.text("world")?,
                players: p.texts("players")?,
                net_http: p.texts("netHttp")?,
                channels: p.bool("channels")?,
                native: p.bool("native")?,
            },
            source_url: self.opt_text("sourceUrl")?,
            index_listed: self.bool("indexListed")?,
        })
    }
}

fn as_uint(v: &Value, key: &'static str) -> Result<u64, Error> {
    match v {
        Value::Uint(n) => Ok(*n),
        _ => Err(Error::Type(key)),
    }
}

fn as_text(v: &Value, key: &'static str) -> Result<String, Error> {
    match v {
        Value::Text(s) => Ok(s.clone()),
        _ => Err(Error::Type(key)),
    }
}

fn as_hash(v: &Value, key: &'static str) -> Result<Hash, Error> {
    match v {
        Value::Bytes(b) => b.as_slice().try_into().map_err(|_| Error::Type(key)),
        _ => Err(Error::Type(key)),
    }
}

/// `modsHash`: SHA-256 over the lines `<id>@<version>:<sha256 lowercase hex>\n`, sorted bytewise, for every
/// `MOD_LIST` entry with `clientRequired = true` (pass only those). No mods gives the SHA-256 of the empty string.
pub fn mods_hash<'a>(mods: impl IntoIterator<Item = (&'a str, &'a str, &'a Hash)>) -> Hash {
    let mut lines: Vec<String> = mods.into_iter().map(|(id, v, h)| format!("{id}@{v}:{}\n", hex(h))).collect();
    lines.sort();
    Sha256::digest(lines.concat().as_bytes()).into()
}

/// Lowercase hex of a hash (used for `acceptedHash` inside the 0x1f `bhcModLoader` plist dict).
pub fn hex(h: &Hash) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// The vanilla rejection packet sent to a client without the mod loader: `0x26 0x03`.
pub fn vanilla_reject_packet() -> [u8; 2] {
    [0x26, REJECT_CLIENT_TOO_OLD]
}
