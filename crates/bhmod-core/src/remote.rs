//! Cross-side service calls (API 0.3.1): a client mod calls a service a server mod provides, and the other way round,
//! over the 0xBC `SERVICE_CALL` / `SERVICE_RESULT` ops.
//!
//! The caller side is asynchronous. `services.call-remote` returns a call id at once; the answer (or the failure)
//! arrives later as the custom event `<mod-id>:service-result` to the calling mod, whatever the language. The
//! answering side is synchronous: [`crate::ModHost::remote_call_in`] runs the provider's `call-service` under the
//! provider's own time budget and returns the result for the host program to send back.
//!
//! This module holds the data types, the bookkeeping of open calls and the per-peer rate limiter. The wiring to the
//! mods is in [`crate::host`].

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use bhmod_proto::{service_error as wire, ServiceCall, ServiceFailure};

use crate::types::{ApiError, ServiceError};
use crate::value::Value;

/// Timeout of a call that asks for none (`timeout-ms = 0`).
pub const DEFAULT_TIMEOUT_MS: u32 = 5_000;
/// Longest timeout a call may ask for.
pub const MAX_TIMEOUT_MS: u32 = 30_000;
/// Largest arguments / result in bytes (the protocol's `MAX_SERVICE_PAYLOAD`).
pub const MAX_PAYLOAD: usize = 32 * 1024;
/// Longest error text a provider's failure carries to the other side, in bytes.
pub const MAX_ERROR: usize = 256;
/// Longest service or method name in bytes.
pub const MAX_NAME: usize = 64;

/// Where a remote call goes, and where one came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RemoteTarget {
    /// The server (from a client mod).
    Server,
    /// One player's client (from a server mod), by player id.
    Player(String),
}

/// One call on its way out, as the [`crate::GameBridge`] sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCall {
    /// The caller's id for the call; the answer carries it back.
    pub id: u32,
    /// The side the call goes to.
    pub target: RemoteTarget,
    /// The calling mod.
    pub mod_id: String,
    /// Service name.
    pub service: String,
    /// Semver requirement on the service version.
    pub version_req: String,
    /// Method name.
    pub method: String,
    /// Canonical CBOR array of the arguments.
    pub args: Vec<u8>,
}

/// Who is calling the service that is running now (`services.caller`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCaller {
    /// The mod that made the call on the other side.
    pub mod_id: String,
    /// The calling player when a client called a server mod; `None` when the server called a client mod.
    pub player: Option<String>,
}

/// Why a remote call failed. The first group comes back from the answering side (and has a wire code), the second
/// group never leaves this side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    /// No provider with that name is loaded there.
    NotFound,
    /// Providers exist but none matches the version requirement.
    VersionMismatch,
    /// The service or the method is not exposed to this side.
    NotPermitted,
    /// Rate limited or too many calls open; try later.
    Busy,
    /// The provider returned an error of its own.
    ProviderError(String),
    /// The provider trapped or ran over its time budget.
    Trap,
    /// The arguments or the result were not valid canonical CBOR for the call.
    BadValue(String),
    /// The arguments or the result were too large.
    TooLarge,
    /// The answering side could not run the call (also: an error code this side does not know).
    Internal(String),
    /// There is nobody to ask: not connected to a modded server, or the player has no mod loader.
    Unavailable,
    /// No answer within the timeout.
    Timeout,
    /// The other side went away before it answered.
    Disconnected,
}

impl RemoteError {
    /// The error kind as the `service-result` event and the SDKs name it.
    pub fn kind(&self) -> &'static str {
        match self {
            RemoteError::NotFound => "not-found",
            RemoteError::VersionMismatch => "version-mismatch",
            RemoteError::NotPermitted => "not-permitted",
            RemoteError::Busy => "busy",
            RemoteError::ProviderError(_) => "provider-error",
            RemoteError::Trap => "trap",
            RemoteError::BadValue(_) => "bad-value",
            RemoteError::TooLarge => "too-large",
            RemoteError::Internal(_) => "internal",
            RemoteError::Unavailable => "unavailable",
            RemoteError::Timeout => "timeout",
            RemoteError::Disconnected => "disconnected",
        }
    }

    /// The detail text (empty when the kind says it all).
    pub fn message(&self) -> &str {
        match self {
            RemoteError::ProviderError(m) | RemoteError::BadValue(m) | RemoteError::Internal(m) => m,
            _ => "",
        }
    }

    /// The `SERVICE_RESULT` code of a failure that travels (`None` for the local-only kinds).
    pub fn wire_code(&self) -> Option<u8> {
        Some(match self {
            RemoteError::NotFound => wire::NOT_FOUND,
            RemoteError::VersionMismatch => wire::VERSION_MISMATCH,
            RemoteError::NotPermitted => wire::NOT_PERMITTED,
            RemoteError::Busy => wire::BUSY,
            RemoteError::ProviderError(_) => wire::PROVIDER_ERROR,
            RemoteError::Trap => wire::TRAP,
            RemoteError::BadValue(_) => wire::BAD_VALUE,
            RemoteError::TooLarge => wire::TOO_LARGE,
            RemoteError::Internal(_) => wire::INTERNAL,
            RemoteError::Unavailable | RemoteError::Timeout | RemoteError::Disconnected => return None,
        })
    }

    /// The failure a `SERVICE_RESULT` carries; unknown codes read as [`RemoteError::Internal`].
    pub fn from_wire(c: u8, message: String) -> RemoteError {
        match c {
            wire::NOT_FOUND => RemoteError::NotFound,
            wire::VERSION_MISMATCH => RemoteError::VersionMismatch,
            wire::NOT_PERMITTED => RemoteError::NotPermitted,
            wire::BUSY => RemoteError::Busy,
            wire::PROVIDER_ERROR => RemoteError::ProviderError(message),
            wire::TRAP => RemoteError::Trap,
            wire::BAD_VALUE => RemoteError::BadValue(message),
            wire::TOO_LARGE => RemoteError::TooLarge,
            _ => RemoteError::Internal(message),
        }
    }

    /// The `(code, message)` to put on the wire. Local-only kinds go out as `INTERNAL` (an answering side never has
    /// them, so this is for completeness).
    pub fn to_wire(&self) -> (u8, String) {
        (self.wire_code().unwrap_or(9), self.message().to_string())
    }
}

impl From<ServiceFailure> for RemoteError {
    fn from(f: ServiceFailure) -> RemoteError {
        RemoteError::from_wire(f.code, f.message)
    }
}

impl From<&RemoteError> for ServiceFailure {
    fn from(e: &RemoteError) -> ServiceFailure {
        let (code, message) = e.to_wire();
        ServiceFailure { code, message }
    }
}

impl From<&RemoteCall> for ServiceCall {
    fn from(c: &RemoteCall) -> ServiceCall {
        ServiceCall {
            id: c.id,
            mod_id: c.mod_id.clone(),
            service: c.service.clone(),
            version_req: c.version_req.clone(),
            method: c.method.clone(),
            args: c.args.clone(),
        }
    }
}

impl From<ServiceError> for RemoteError {
    fn from(e: ServiceError) -> RemoteError {
        match e {
            ServiceError::NotFound => RemoteError::NotFound,
            ServiceError::VersionMismatch => RemoteError::VersionMismatch,
            ServiceError::NotPermitted => RemoteError::NotPermitted,
            ServiceError::Reentrant => RemoteError::Busy,
            ServiceError::ProviderError(m) => RemoteError::ProviderError(m),
            ServiceError::Trap => RemoteError::Trap,
            ServiceError::BadValue(m) => RemoteError::BadValue(m),
        }
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.message() {
            "" => f.write_str(self.kind()),
            m => write!(f, "{}: {m}", self.kind()),
        }
    }
}

impl std::error::Error for RemoteError {}

/// Limits of cross-side calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteLimits {
    /// Calls one peer may make in a burst (token bucket size).
    pub burst: u32,
    /// Calls per second one peer may keep up (token refill rate).
    pub per_second: u32,
    /// Most open outgoing calls per mod.
    pub max_pending: usize,
}

impl Default for RemoteLimits {
    fn default() -> Self {
        RemoteLimits { burst: 20, per_second: 10, max_pending: 16 }
    }
}

/// An open outgoing call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The calling mod (slot index).
    pub owner: usize,
    /// Who was asked.
    pub target: RemoteTarget,
    /// When the call times out.
    pub deadline: Instant,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Open outgoing calls, the per-peer rate limiter and the caller of the running incoming call.
#[derive(Debug, Default)]
pub struct Remote {
    limits: RemoteLimits,
    next_id: u32,
    pending: BTreeMap<u32, Pending>,
    buckets: HashMap<RemoteTarget, Bucket>,
    /// The caller of the outermost incoming call that is still running.
    pub caller: Option<RemoteCaller>,
}

/// Buckets idle for this long are dropped when the table is large.
const IDLE: Duration = Duration::from_secs(60);
/// Table size above which idle buckets are pruned.
const PRUNE_ABOVE: usize = 1024;

impl Remote {
    /// An empty table with `limits`.
    pub fn new(limits: RemoteLimits) -> Remote {
        Remote { limits, ..Remote::default() }
    }

    /// Opens a call of `owner`: the id to put on the wire. `LimitExceeded` when the mod has too many open.
    pub fn begin(&mut self, owner: usize, target: RemoteTarget, deadline: Instant) -> Result<u32, ApiError> {
        if self.pending.values().filter(|p| p.owner == owner).count() >= self.limits.max_pending {
            return Err(ApiError::LimitExceeded("open remote calls".into()));
        }
        let mut id = self.next_id;
        while id == 0 || self.pending.contains_key(&id) {
            id = id.wrapping_add(1);
        }
        self.next_id = id.wrapping_add(1);
        self.pending.insert(id, Pending { owner, target, deadline });
        Ok(id)
    }

    /// Closes the call `id` if `from` is who it was sent to (nobody else can answer it).
    pub fn answer(&mut self, id: u32, from: &RemoteTarget) -> Option<Pending> {
        if self.pending.get(&id).is_some_and(|p| &p.target == from) {
            self.pending.remove(&id)
        } else {
            None
        }
    }

    /// Closes the call `id` of `owner` without an answer. False when it is not open.
    pub fn cancel(&mut self, owner: usize, id: u32) -> bool {
        if self.pending.get(&id).is_some_and(|p| p.owner == owner) {
            self.pending.remove(&id);
            return true;
        }
        false
    }

    /// Takes the calls that time out at `now`, in id order.
    pub fn expired(&mut self, now: Instant) -> Vec<(u32, Pending)> {
        self.take_where(|p| p.deadline <= now)
    }

    /// Takes the calls sent to `target`.
    pub fn gone(&mut self, target: &RemoteTarget) -> Vec<(u32, Pending)> {
        self.take_where(|p| &p.target == target)
    }

    /// Forgets the calls of a mod that was disabled.
    pub fn remove_owner(&mut self, owner: usize) {
        self.pending.retain(|_, p| p.owner != owner);
    }

    /// True when some call is open.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn take_where(&mut self, f: impl Fn(&Pending) -> bool) -> Vec<(u32, Pending)> {
        let ids: Vec<u32> = self.pending.iter().filter(|(_, p)| f(p)).map(|(id, _)| *id).collect();
        ids.into_iter().filter_map(|id| self.pending.remove(&id).map(|p| (id, p))).collect()
    }

    /// Rate limit for incoming calls: takes a token of `from`'s bucket. False when it is empty.
    pub fn allow(&mut self, from: &RemoteTarget, now: Instant) -> bool {
        if self.buckets.len() > PRUNE_ABOVE {
            self.buckets.retain(|_, b| now.saturating_duration_since(b.last) < IDLE);
        }
        let burst = f64::from(self.limits.burst);
        let rate = f64::from(self.limits.per_second);
        let b = self.buckets.entry(from.clone()).or_insert(Bucket { tokens: burst, last: now });
        let dt = now.saturating_duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + dt * rate).min(burst);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The payload of the `<mod-id>:service-result` event. `value` is the (already validated) canonical CBOR result.
pub fn result_payload(id: u32, outcome: &Result<Vec<u8>, RemoteError>) -> Value {
    let mut m = vec![("id".to_string(), Value::Int(i64::from(id)))];
    match outcome {
        Ok(bytes) => {
            m.push(("ok".into(), Value::Bool(true)));
            m.push(("value".into(), Value::decode(bytes).unwrap_or(Value::Null)));
        }
        Err(e) => {
            m.push(("ok".into(), Value::Bool(false)));
            m.push((
                "error".into(),
                Value::Map(vec![
                    ("kind".into(), Value::Text(e.kind().into())),
                    ("message".into(), Value::Text(e.message().into())),
                ]),
            ));
        }
    }
    Value::Map(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn error_names_and_wire_codes() {
        let all = [
            (RemoteError::NotFound, "not-found", 1),
            (RemoteError::VersionMismatch, "version-mismatch", 2),
            (RemoteError::NotPermitted, "not-permitted", 3),
            (RemoteError::Busy, "busy", 4),
            (RemoteError::ProviderError("p".into()), "provider-error", 5),
            (RemoteError::Trap, "trap", 6),
            (RemoteError::BadValue("b".into()), "bad-value", 7),
            (RemoteError::TooLarge, "too-large", 8),
            (RemoteError::Internal("i".into()), "internal", 9),
        ];
        for (e, kind, code) in all {
            assert_eq!(e.kind(), kind);
            assert_eq!(e.wire_code(), Some(code));
            let (c, m) = e.to_wire();
            assert_eq!(RemoteError::from_wire(c, m), e);
        }
        assert_eq!(RemoteError::from_wire(200, "x".into()), RemoteError::Internal("x".into()));
        for (e, kind) in [
            (RemoteError::Unavailable, "unavailable"),
            (RemoteError::Timeout, "timeout"),
            (RemoteError::Disconnected, "disconnected"),
        ] {
            assert_eq!((e.kind(), e.wire_code(), e.to_wire().0), (kind, None, 9));
        }
        assert_eq!(RemoteError::Trap.to_string(), "trap");
        assert_eq!(RemoteError::ProviderError("no".into()).to_string(), "provider-error: no");
        let _: &dyn std::error::Error = &RemoteError::Trap;
    }

    #[test]
    fn wire_conversions() {
        let f = ServiceFailure::from(&RemoteError::ProviderError("p".into()));
        assert_eq!(f, ServiceFailure { code: wire::PROVIDER_ERROR, message: "p".into() });
        assert_eq!(RemoteError::from(f), RemoteError::ProviderError("p".into()));
        assert_eq!(ServiceFailure::from(&RemoteError::Timeout).code, wire::INTERNAL);
        let call = RemoteCall {
            id: 7,
            target: RemoteTarget::Server,
            mod_id: "m".into(),
            service: "s".into(),
            version_req: "^1".into(),
            method: "x".into(),
            args: vec![0x80],
        };
        let wire = ServiceCall::from(&call);
        assert_eq!((wire.id, wire.mod_id.as_str(), wire.service.as_str()), (7, "m", "s"));
        assert_eq!((wire.version_req.as_str(), wire.method.as_str(), wire.args), ("^1", "x", vec![0x80]));
    }

    #[test]
    fn service_errors_map() {
        assert_eq!(RemoteError::from(ServiceError::NotFound), RemoteError::NotFound);
        assert_eq!(RemoteError::from(ServiceError::VersionMismatch), RemoteError::VersionMismatch);
        assert_eq!(RemoteError::from(ServiceError::NotPermitted), RemoteError::NotPermitted);
        assert_eq!(RemoteError::from(ServiceError::Reentrant), RemoteError::Busy);
        assert_eq!(RemoteError::from(ServiceError::ProviderError("p".into())), RemoteError::ProviderError("p".into()));
        assert_eq!(RemoteError::from(ServiceError::Trap), RemoteError::Trap);
        assert_eq!(RemoteError::from(ServiceError::BadValue("b".into())), RemoteError::BadValue("b".into()));
    }

    #[test]
    fn pending_calls() {
        let now = t0();
        let mut r = Remote::new(RemoteLimits { max_pending: 2, ..RemoteLimits::default() });
        let p1 = RemoteTarget::Player("p1".into());
        let a = r.begin(0, RemoteTarget::Server, now + Duration::from_secs(1)).unwrap();
        let b = r.begin(0, p1.clone(), now + Duration::from_secs(3)).unwrap();
        assert_ne!(a, b);
        assert_eq!(r.begin(0, RemoteTarget::Server, now), Err(ApiError::LimitExceeded("open remote calls".into())));
        // Another mod has its own allowance.
        let c = r.begin(1, RemoteTarget::Server, now + Duration::from_secs(2)).unwrap();
        assert!(r.has_pending());
        // Only the peer the call went to can answer it.
        assert!(r.answer(a, &p1).is_none());
        assert_eq!(r.answer(a, &RemoteTarget::Server).map(|p| p.owner), Some(0));
        assert!(r.answer(a, &RemoteTarget::Server).is_none());
        // Cancel is for the owner only.
        assert!(!r.cancel(1, b));
        assert!(r.cancel(0, b));
        assert!(!r.cancel(0, b));
        assert!(!r.cancel(0, 999));
        assert_eq!(r.expired(now).len(), 0);
        let late = r.expired(now + Duration::from_secs(2));
        assert_eq!(late.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![c]);
        let d = r.begin(1, p1.clone(), now).unwrap();
        assert_eq!(r.gone(&p1).iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![d]);
        r.begin(1, RemoteTarget::Server, now).unwrap();
        r.remove_owner(1);
        assert!(!r.has_pending());
    }

    #[test]
    fn ids_skip_zero_and_open_calls() {
        let now = t0();
        let mut r = Remote::new(RemoteLimits { max_pending: 100, ..RemoteLimits::default() });
        r.next_id = u32::MAX;
        assert_eq!(r.begin(0, RemoteTarget::Server, now).unwrap(), u32::MAX);
        // 0 is never an id; 1 is next.
        assert_eq!(r.begin(0, RemoteTarget::Server, now).unwrap(), 1);
        r.next_id = 1;
        assert_eq!(r.begin(0, RemoteTarget::Server, now).unwrap(), 2);
    }

    #[test]
    fn token_bucket() {
        let now = t0();
        let mut r = Remote::new(RemoteLimits { burst: 3, per_second: 2, max_pending: 1 });
        let a = RemoteTarget::Player("a".into());
        let b = RemoteTarget::Player("b".into());
        assert!((0..3).all(|_| r.allow(&a, now)));
        assert!(!r.allow(&a, now));
        // Another peer has its own bucket.
        assert!(r.allow(&b, now));
        // Half a second refills one token at 2 per second.
        assert!(r.allow(&a, now + Duration::from_millis(500)));
        assert!(!r.allow(&a, now + Duration::from_millis(500)));
        // The bucket never holds more than the burst.
        let later = now + Duration::from_secs(100);
        assert!((0..3).all(|_| r.allow(&a, later)));
        assert!(!r.allow(&a, later));
    }

    #[test]
    fn idle_buckets_are_pruned() {
        let now = t0();
        let mut r = Remote::new(RemoteLimits { burst: 1, per_second: 1, max_pending: 1 });
        for i in 0..=PRUNE_ABOVE {
            r.allow(&RemoteTarget::Player(format!("p{i}")), now);
        }
        assert_eq!(r.buckets.len(), PRUNE_ABOVE + 1);
        r.allow(&RemoteTarget::Server, now + IDLE * 2);
        assert_eq!(r.buckets.len(), 1);
    }

    #[test]
    fn result_payloads() {
        let ok = result_payload(4, &Ok(Value::Int(7).encode()));
        assert_eq!(
            ok,
            Value::Map(vec![
                ("id".into(), Value::Int(4)),
                ("ok".into(), Value::Bool(true)),
                ("value".into(), Value::Int(7))
            ])
        );
        let err = result_payload(5, &Err(RemoteError::ProviderError("no".into())));
        assert_eq!(
            err,
            Value::Map(vec![
                ("id".into(), Value::Int(5)),
                ("ok".into(), Value::Bool(false)),
                (
                    "error".into(),
                    Value::Map(vec![
                        ("kind".into(), Value::Text("provider-error".into())),
                        ("message".into(), Value::Text("no".into()))
                    ])
                ),
            ])
        );
        // Bytes that are not a value cannot happen (the host validates them) and read as null.
        let Value::Map(m) = result_payload(1, &Ok(vec![0xff])) else { unreachable!() };
        assert_eq!(m[2], ("value".into(), Value::Null));
    }
}
