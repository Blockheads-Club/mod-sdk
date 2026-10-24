//! The modded join of one connection, as a sans-IO state machine.
//!
//! The host calls [`Session::contact`] when the server's first packet (0x23) arrives, [`Session::join_requested`]
//! when the game is about to send its 0x1f (the host holds the 0x1f until [`Output::Join`]), [`Session::receive`]
//! for every 0xBC packet, [`Session::tick`] regularly, [`Session::decide`] with the player's answer to
//! [`Output::Prompt`] and [`Session::loaded`] once the packages of [`Output::Load`] run. Every call returns the
//! [`Output`]s to act on, in order. All times are milliseconds of one monotonic clock.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use bhmod_content::registry::Wire;
use bhmod_manifest::Manifest;
use bhmod_proto::{
    file_error, Channel, Cosmetics, FileRequest, Hash, Hello, Message, ModInfo, ModList, PlayerCount, Ready,
    RejectText, ServerHello, ServiceCall, ServiceResult, CHUNK_SIZE, HELLO_TIMEOUT_MS, MAX_CHANNEL_PAYLOAD, PROTO,
};

use crate::cache::PackageCache;
use crate::verify::{check_offered_with, Caps, Refusal};

/// Acknowledge after this many chunks (the server keeps 8 in flight).
pub const ACK_EVERY: u32 = 4;

/// Behaviour and identity of the loader.
#[derive(Debug, Clone)]
pub struct Options {
    /// `HELLO.loader`.
    pub loader: String,
    /// `HELLO.platform`.
    pub platform: String,
    /// `HELLO.api`.
    pub api: String,
    /// `HELLO.runtimes`.
    pub runtimes: Vec<String>,
    /// How long to wait for `SERVER_HELLO`.
    pub hello_timeout_ms: u64,
    /// How long to wait for `MOD_LIST`.
    pub list_timeout_ms: u64,
    /// A transfer with no chunk for this long fails.
    pub stall_timeout_ms: u64,
    /// Wait after `FILE_ERROR` BUSY before asking again.
    pub busy_retry_ms: u64,
    /// Accept packages with native code (the host checked the public index and the player confirmed). Off by
    /// default: the server can never push native code on its own.
    pub allow_native: bool,
    /// Packages (by SHA-256) the host trusts to carry native code: the player installed exactly this file from the
    /// public index and confirmed the red warning. A server listing native code with another hash is refused.
    pub native_trusted: Vec<Hash>,
}

impl Options {
    /// Protocol defaults for a loader.
    pub fn new(loader: &str, platform: &str, api: &str, runtimes: &[&str]) -> Options {
        Options {
            loader: loader.into(),
            platform: platform.into(),
            api: api.into(),
            runtimes: runtimes.iter().map(|s| s.to_string()).collect(),
            hello_timeout_ms: HELLO_TIMEOUT_MS,
            list_timeout_ms: 10_000,
            stall_timeout_ms: 30_000,
            busy_retry_ms: 1_000,
            allow_native: false,
            native_trusted: Vec::new(),
        }
    }
}

/// One listed mod as the prompt shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The `MOD_LIST` entry.
    pub info: ModInfo,
    /// Already in the cache (no download).
    pub cached: bool,
    /// Has a client half (server-only mods are listed for information).
    pub client: bool,
}

/// A checked package ready to run.
#[derive(Debug, Clone)]
pub struct Accepted {
    /// The `MOD_LIST` entry.
    pub info: ModInfo,
    /// The package in the cache.
    pub path: PathBuf,
    /// Its manifest.
    pub manifest: Manifest,
}

/// Why the modded join stopped. The host disconnects (or lets the server reject) and shows the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The player declined mods the server requires.
    Declined,
    /// The server asked for a mod this loader refuses (native code, newer API, ...).
    Refused {
        /// Mod id.
        id: String,
        /// Why.
        refusal: Refusal,
    },
    /// The server answered `FILE_ERROR`.
    Download {
        /// Mod id.
        id: String,
        /// The `FILE_ERROR` code.
        code: u64,
    },
    /// No data for too long.
    Stalled(String),
    /// The downloaded file does not have the listed SHA-256.
    HashMismatch(String),
    /// The server broke the protocol (the text says how).
    Protocol(String),
    /// The cache could not be written.
    Disk(String),
    /// The mods did not start.
    LoadFailed(String),
}

impl Failure {
    /// A stable code for translated UI texts.
    pub fn code(&self) -> &'static str {
        match self {
            Failure::Declined => "declined",
            Failure::Refused { .. } => "refused",
            Failure::Download { .. } => "download",
            Failure::Stalled(_) => "stalled",
            Failure::HashMismatch(_) => "hash-mismatch",
            Failure::Protocol(_) => "protocol",
            Failure::Disk(_) => "disk",
            Failure::LoadFailed(_) => "load-failed",
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Declined => write!(f, "you declined the server's mods"),
            Failure::Refused { id, refusal } => write!(f, "{id}: {refusal}"),
            Failure::Download { id, code } => write!(f, "{id}: the server could not send the file (error {code})"),
            Failure::Stalled(id) => write!(f, "{id}: the download stopped"),
            Failure::HashMismatch(id) => write!(f, "{id}: the file check failed (SHA-256)"),
            Failure::Protocol(t) => write!(f, "the server sent something unexpected: {t}"),
            Failure::Disk(t) => write!(f, "could not save the mod: {t}"),
            Failure::LoadFailed(t) => write!(f, "the mods did not start: {t}"),
        }
    }
}

/// What the host does next.
#[derive(Debug, Clone)]
pub enum Output {
    /// Send this packet to the server (reliable).
    Send(Vec<u8>),
    /// Ask the player. `required`: joining needs the client mods.
    Prompt {
        /// The server's `SERVER_HELLO.clientRequired`.
        required: bool,
        /// Every listed mod.
        mods: Vec<Offer>,
        /// Bytes to download on accept.
        download: u64,
    },
    /// Download progress of file `index` of `count`.
    Progress {
        /// Mod id.
        id: String,
        /// 0-based file index.
        index: usize,
        /// Files to download.
        count: usize,
        /// Bytes held of this file.
        done: u64,
        /// File size.
        size: u64,
    },
    /// Start these packages (client side), then call [`Session::loaded`].
    Load(Vec<Accepted>),
    /// Send the held 0x1f now; with `Some(hash)` it carries `bhcModLoader{acceptedHash: hash, proto: 1}`.
    Join(Option<Hash>),
    /// The modded join stopped.
    Failed(Failure),
    /// The server's `REJECT_TEXT` (the vanilla 0x26 follows): show it instead of the vanilla text.
    Reject(RejectText),
    /// Which cosmetics players wear (`COSMETICS`, only after the join).
    Cosmetics(Cosmetics),
    /// A channel message for the running mods.
    Channel {
        /// `<mod-id>:<name>`.
        name: String,
        /// Payload.
        data: Vec<u8>,
    },
    /// The server calls a service of a client mod (`SERVICE_CALL`); answer with [`Session::service_result`].
    ServiceCall(ServiceCall),
    /// The server's answer to a call of a client mod (`SERVICE_RESULT`).
    ServiceResult(ServiceResult),
}

#[derive(Debug, Clone)]
struct Transfer {
    index: usize,
    have: u64,
    since_ack: u32,
    deadline: u64,
    busy_until: Option<u64>,
}

#[derive(Debug, Clone)]
enum State {
    Idle,
    AwaitHello { deadline: u64 },
    Vanilla,
    AwaitList { hello: ServerHello, deadline: u64 },
    Prompt { hello: ServerHello, list: ModList },
    Download { hello: ServerHello, list: ModList, selected: Vec<usize>, wanted: Vec<usize>, cur: Transfer },
    Loading { hello: ServerHello },
    Modded,
    Failed,
}

/// The modded join of one connection.
#[derive(Debug)]
pub struct Session {
    opts: Options,
    cache: PackageCache,
    caps: Caps,
    state: State,
    join_wanted: bool,
    join: Option<Option<Hash>>,
    joined: bool,
    channels: BTreeMap<String, u16>,
    registry: Wire,
    players: Option<PlayerCount>,
}

fn client_side(m: &ModInfo) -> bool {
    m.sides.iter().any(|s| s == "client")
}

impl Session {
    /// A session for one connection.
    pub fn new(opts: Options, cache: PackageCache, caps: Caps) -> Session {
        Session {
            opts,
            cache,
            caps,
            state: State::Idle,
            join_wanted: false,
            join: None,
            joined: false,
            channels: BTreeMap::new(),
            registry: Wire::new(),
            players: None,
        }
    }

    /// The player count the server put into `SERVER_HELLO`, if it did.
    pub fn players(&self) -> Option<PlayerCount> {
        self.players
    }

    /// The content id map of the server's `MOD_LIST` (empty until it arrived or without content).
    pub fn registry(&self) -> &Wire {
        &self.registry
    }

    /// A short state name for logs.
    pub fn state_name(&self) -> &'static str {
        match self.state {
            State::Idle => "idle",
            State::AwaitHello { .. } => "await-hello",
            State::Vanilla => "vanilla",
            State::AwaitList { .. } => "await-list",
            State::Prompt { .. } => "prompt",
            State::Download { .. } => "download",
            State::Loading { .. } => "loading",
            State::Modded => "modded",
            State::Failed => "failed",
        }
    }

    /// True once the server is known to be modded and the player's client runs its mods (channels work).
    pub fn is_modded(&self) -> bool {
        matches!(self.state, State::Modded)
    }

    /// The `HELLO` this loader sends.
    pub fn hello(&self) -> Hello {
        Hello {
            proto: PROTO,
            loader: self.opts.loader.clone(),
            platform: self.opts.platform.clone(),
            api: self.opts.api.clone(),
            runtimes: self.opts.runtimes.clone(),
            cached: self.cache.cached(),
        }
    }

    /// The server's first packet (0x23) arrived: send `HELLO` (once per connection).
    pub fn contact(&mut self, now: u64) -> Vec<Output> {
        if !matches!(self.state, State::Idle) {
            return Vec::new();
        }
        self.state = State::AwaitHello { deadline: now + self.opts.hello_timeout_ms };
        vec![Output::Send(Message::Hello(self.hello()).encode())]
    }

    /// The game wants to send its 0x1f. Returns [`Output::Join`] at once when the outcome is known, else later.
    pub fn join_requested(&mut self) -> Vec<Output> {
        self.join_wanted = true;
        if matches!(self.state, State::Idle) {
            // No 0x23 seen (should not happen): behave like vanilla.
            self.state = State::Vanilla;
            self.join = Some(None);
        }
        self.release()
    }

    fn release(&mut self) -> Vec<Output> {
        match self.join {
            Some(j) if self.join_wanted && !self.joined => {
                self.joined = true;
                vec![Output::Join(j)]
            }
            _ => Vec::new(),
        }
    }

    fn fail(&mut self, f: Failure) -> Vec<Output> {
        self.state = State::Failed;
        vec![Output::Failed(f)]
    }

    fn modded(&mut self, hash: Hash) -> Vec<Output> {
        self.state = State::Modded;
        self.join = Some(Some(hash));
        self.release()
    }

    /// Time passed: timeouts and retries.
    pub fn tick(&mut self, now: u64) -> Vec<Output> {
        match &mut self.state {
            State::AwaitHello { deadline } if now >= *deadline => {
                self.state = State::Vanilla;
                self.join = Some(None);
                self.release()
            }
            State::AwaitList { deadline, .. } if now >= *deadline => self.fail(Failure::Protocol("no mod list".into())),
            State::Download { list, cur, .. } => {
                if cur.busy_until.is_some_and(|t| now >= t) {
                    cur.busy_until = None;
                    cur.deadline = now + self.opts.stall_timeout_ms;
                    let sha = list.mods[cur.index].sha256;
                    let offset = cur.have;
                    return vec![Output::Send(Message::FileRequest(FileRequest { sha256: sha, offset }).encode())];
                }
                if cur.busy_until.is_none() && now >= cur.deadline {
                    let id = list.mods[cur.index].id.clone();
                    return self.fail(Failure::Stalled(id));
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// A 0xBC packet from the server.
    pub fn receive(&mut self, packet: &[u8], now: u64) -> Vec<Output> {
        let msg = match Message::decode(packet) {
            Ok(m) => m,
            // Unknown ops and malformed packets are dropped.
            Err(_) => return Vec::new(),
        };
        match msg {
            Message::ServerHello(h) => self.on_server_hello(h, now),
            Message::ModList(l) => self.on_list(l),
            Message::FileChunk(c) => self.on_chunk(c.sha256, c.offset, c.total, &c.data, now),
            Message::FileError(e) => self.on_file_error(e.sha256, e.code, now),
            Message::RejectText(r) => {
                if matches!(self.state, State::AwaitHello { .. }) {
                    // Protocol mismatch: the 0x26 follows; join without the key so the game shows the rejection.
                    self.state = State::Failed;
                    self.join = Some(None);
                    let mut out = vec![Output::Reject(r)];
                    out.extend(self.release());
                    return out;
                }
                vec![Output::Reject(r)]
            }
            Message::ChannelRegister(c) => {
                self.channels.insert(c.name, c.id);
                Vec::new()
            }
            Message::Cosmetics(c) if self.is_modded() => vec![Output::Cosmetics(c)],
            Message::ServiceCall(c) if self.is_modded() => vec![Output::ServiceCall(c)],
            Message::ServiceResult(r) if self.is_modded() => vec![Output::ServiceResult(r)],
            Message::Channel(c) if self.is_modded() => {
                match self.channels.iter().find(|(_, id)| **id == c.channel_id) {
                    Some((name, _)) => vec![Output::Channel { name: name.clone(), data: c.payload }],
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn on_server_hello(&mut self, h: ServerHello, now: u64) -> Vec<Output> {
        if !matches!(self.state, State::AwaitHello { .. }) {
            return Vec::new();
        }
        self.players = h.players;
        if h.count == 0 {
            return self.modded(h.mods_hash);
        }
        self.state = State::AwaitList { hello: h, deadline: now + self.opts.list_timeout_ms };
        vec![Output::Send(Message::ListRequest.encode())]
    }

    fn on_list(&mut self, list: ModList) -> Vec<Output> {
        let State::AwaitList { hello, .. } = &self.state else { return Vec::new() };
        let hello = hello.clone();
        self.registry = list.registry.clone();
        for c in &list.channels {
            self.channels.insert(c.name.clone(), c.id);
        }
        let mods: Vec<Offer> = list
            .mods
            .iter()
            .map(|m| Offer { info: m.clone(), cached: self.cache.get(&m.sha256).is_some(), client: client_side(m) })
            .collect();
        if !mods.iter().any(|o| o.client) {
            return self.modded(hello.mods_hash);
        }
        let download = mods.iter().filter(|o| o.client && !o.cached).map(|o| o.info.size).sum();
        self.state = State::Prompt { hello: hello.clone(), list };
        vec![Output::Prompt { required: hello.client_required, mods, download }]
    }

    /// The player's answer to [`Output::Prompt`]: `true` takes every listed client mod.
    pub fn decide(&mut self, accept: bool, now: u64) -> Vec<Output> {
        self.choose(accept, None, now)
    }

    /// Like [`Session::decide`] when the player picked among the optional mods: with `accept`, the required client
    /// mods plus the optional ones whose id is in `optional` are downloaded and run (`READY.loaded` tells the
    /// server which). Declining optional mods never changes `modsHash`.
    pub fn decide_subset(&mut self, accept: bool, optional: &[String], now: u64) -> Vec<Output> {
        self.choose(accept, Some(optional), now)
    }

    fn choose(&mut self, accept: bool, optional: Option<&[String]>, now: u64) -> Vec<Output> {
        let State::Prompt { hello, list } = &self.state else { return Vec::new() };
        let (hello, list) = (hello.clone(), list.clone());
        let selected: Vec<usize> = (0..list.mods.len())
            .filter(|&i| {
                let m = &list.mods[i];
                client_side(m) && (m.client_required || optional.is_none_or(|o| o.contains(&m.id)))
            })
            .collect();
        if !accept {
            if hello.client_required {
                return self.fail(Failure::Declined);
            }
            // Only optional client mods: join as a modded client without them.
            return self.modded(hello.mods_hash);
        }
        if selected.is_empty() {
            return self.modded(hello.mods_hash);
        }
        if !self.opts.allow_native {
            if let Some(m) = selected
                .iter()
                .map(|&i| &list.mods[i])
                .find(|m| m.permissions.native && !self.native_trusted(&m.sha256))
            {
                return self.fail(Failure::Refused { id: m.id.clone(), refusal: Refusal::Native });
            }
        }
        let wanted: Vec<usize> =
            selected.iter().copied().filter(|&i| self.cache.get(&list.mods[i].sha256).is_none()).collect();
        self.next_file(hello, list, selected, wanted, 0, now)
    }

    /// Starts download `pos` of `wanted`, or checks everything when done.
    fn next_file(
        &mut self,
        hello: ServerHello,
        list: ModList,
        selected: Vec<usize>,
        wanted: Vec<usize>,
        pos: usize,
        now: u64,
    ) -> Vec<Output> {
        let Some(&index) = wanted.get(pos) else { return self.check_all(hello, list, &selected) };
        let m = &list.mods[index];
        let have = self.cache.resume_offset(&m.sha256).min(m.size / CHUNK_SIZE as u64 * CHUNK_SIZE as u64);
        let req = Message::FileRequest(FileRequest { sha256: m.sha256, offset: have }).encode();
        let progress = Output::Progress { id: m.id.clone(), index: pos, count: wanted.len(), done: have, size: m.size };
        let cur = Transfer { index, have, since_ack: 0, deadline: now + self.opts.stall_timeout_ms, busy_until: None };
        self.state = State::Download { hello, list, selected, wanted, cur };
        vec![progress, Output::Send(req)]
    }

    fn on_chunk(&mut self, sha: Hash, offset: u64, total: u64, data: &[u8], now: u64) -> Vec<Output> {
        let State::Download { list, wanted, cur, .. } = &mut self.state else { return Vec::new() };
        let m = &list.mods[cur.index];
        if sha != m.sha256 {
            return Vec::new();
        }
        let (id, size) = (m.id.clone(), m.size);
        let last = offset + data.len() as u64 == total;
        if offset != cur.have || total != size || (!last && data.len() != CHUNK_SIZE) {
            let msg = format!("{id}: chunk at {offset} of {total}, expected {}", cur.have);
            return self.fail(Failure::Protocol(msg));
        }
        if let Err(e) = self.cache.write_part(&sha, offset, data) {
            return self.fail(Failure::Disk(e.to_string()));
        }
        cur.have += data.len() as u64;
        cur.since_ack += 1;
        cur.deadline = now + self.opts.stall_timeout_ms;
        let pos = wanted.iter().position(|&i| i == cur.index).expect("current file is wanted");
        let mut out = vec![Output::Progress { id: id.clone(), index: pos, count: wanted.len(), done: cur.have, size }];
        if !last {
            if cur.since_ack >= ACK_EVERY {
                cur.since_ack = 0;
                out.push(Output::Send(Message::FileRequest(FileRequest { sha256: sha, offset: cur.have }).encode()));
            }
            return out;
        }
        match self.cache.finish(&sha) {
            Ok(true) => {}
            Ok(false) => return self.fail(Failure::HashMismatch(id)),
            Err(e) => return self.fail(Failure::Disk(e.to_string())),
        }
        out.push(Output::Send(Message::FileRequest(FileRequest { sha256: sha, offset: size }).encode()));
        if let State::Download { hello, list, selected, wanted, .. } = std::mem::replace(&mut self.state, State::Idle) {
            out.extend(self.next_file(hello, list, selected, wanted, pos + 1, now));
        }
        out
    }

    fn on_file_error(&mut self, sha: Hash, code: u64, now: u64) -> Vec<Output> {
        let State::Download { list, cur, .. } = &mut self.state else { return Vec::new() };
        let m = &list.mods[cur.index];
        if sha != m.sha256 {
            return Vec::new();
        }
        if code == file_error::BUSY {
            cur.busy_until = Some(now + self.opts.busy_retry_ms);
            return Vec::new();
        }
        let id = m.id.clone();
        self.fail(Failure::Download { id, code })
    }

    fn native_trusted(&self, hash: &Hash) -> bool {
        self.opts.native_trusted.contains(hash)
    }

    fn check_all(&mut self, hello: ServerHello, list: ModList, selected: &[usize]) -> Vec<Output> {
        let mut ok = Vec::new();
        for m in selected.iter().map(|&i| &list.mods[i]) {
            let path = self.cache.get(&m.sha256).expect("downloaded or cached");
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => return self.fail(Failure::Disk(e.to_string())),
            };
            match check_offered_with(
                &bytes,
                &m.id,
                &m.version,
                &self.caps,
                self.opts.allow_native || self.native_trusted(&m.sha256),
            ) {
                Ok(manifest) => ok.push(Accepted { info: m.clone(), path, manifest }),
                Err(refusal) => {
                    if !m.client_required {
                        continue;
                    }
                    return self.fail(Failure::Refused { id: m.id.clone(), refusal });
                }
            }
        }
        self.state = State::Loading { hello };
        vec![Output::Load(ok)]
    }

    /// The packages of [`Output::Load`] started (`Ok` with the `(id, version)` of every running mod) or not.
    pub fn loaded(&mut self, result: Result<Vec<(String, String)>, String>) -> Vec<Output> {
        let State::Loading { hello } = &self.state else { return Vec::new() };
        let hash = hello.mods_hash;
        match result {
            Ok(loaded) => {
                let mut out = vec![Output::Send(Message::Ready(Ready { accepted_hash: hash, loaded }).encode())];
                out.extend(self.modded(hash));
                out
            }
            Err(e) => self.fail(Failure::LoadFailed(e)),
        }
    }

    /// A `SERVICE_CALL` packet (a client mod calls a service of the server's mods), when the session is modded.
    pub fn service_call(&self, call: ServiceCall) -> Option<Vec<u8>> {
        self.is_modded().then(|| Message::ServiceCall(call).encode())
    }

    /// A `SERVICE_RESULT` packet (the answer to a call of the server), when the session is modded.
    pub fn service_result(&self, result: ServiceResult) -> Option<Vec<u8>> {
        self.is_modded().then(|| Message::ServiceResult(result).encode())
    }

    /// A `CHANNEL` packet for a mod channel, when the session is modded and the server announced the channel.
    pub fn channel(&self, name: &str, data: &[u8]) -> Option<Vec<u8>> {
        if !self.is_modded() || data.len() > MAX_CHANNEL_PAYLOAD {
            return None;
        }
        let id = *self.channels.get(name)?;
        Some(Message::Channel(Channel { channel_id: id, payload: data.to_vec() }).encode())
    }
}

#[cfg(test)]
mod tests;
