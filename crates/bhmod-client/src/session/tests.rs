use super::*;
use crate::cache::sha256;
use crate::testutil::{caps, package};
use bhmod_proto::{mods_hash, ChannelInfo, ChannelRegister, FileChunk, FileError, ModPermissions};

fn info(id: &str, bytes: &[u8], required: bool, sides: &[&str]) -> ModInfo {
    ModInfo {
        id: id.into(),
        version: "1.0.0".into(),
        name: [("en".to_string(), id.to_string())].into(),
        description: BTreeMap::new(),
        sides: sides.iter().map(|s| s.to_string()).collect(),
        client_required: required,
        sha256: sha256(bytes),
        size: bytes.len() as u64,
        permissions: ModPermissions { world: "none".into(), channels: true, ..Default::default() },
        source_url: None,
        index_listed: false,
    }
}

struct Rig {
    s: Session,
    _t: tempfile::TempDir,
    cache: PackageCache,
}

fn rig() -> Rig {
    let t = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(t.path().join("cache"));
    let mut o = Options::new("0.1.0", "android-armv7", "0.2.0", &["lua"]);
    o.busy_retry_ms = 100;
    Rig { s: Session::new(o, cache.clone(), caps()), _t: t, cache }
}

fn sent(out: &[Output]) -> Vec<Message> {
    out.iter()
        .filter_map(|o| match o {
            Output::Send(b) => Some(Message::decode(b).unwrap()),
            _ => None,
        })
        .collect()
}

fn hello_pkt(hash: Hash, required: bool, count: u64) -> Vec<u8> {
    Message::ServerHello(ServerHello { proto: 1, mods_hash: hash, client_required: required, count }).encode()
}

fn list_pkt(mods: Vec<ModInfo>) -> Vec<u8> {
    let channels = vec![ChannelInfo { name: "demo:echo".into(), id: 3 }];
    Message::ModList(ModList { mods, channels, registry: BTreeMap::new() }).encode()
}

fn chunks(bytes: &[u8], sha: Hash, from: usize) -> Vec<Vec<u8>> {
    bytes
        .chunks(CHUNK_SIZE)
        .enumerate()
        .skip(from)
        .map(|(i, c)| {
            let offset = (i * CHUNK_SIZE) as u64;
            Message::FileChunk(FileChunk { sha256: sha, offset, total: bytes.len() as u64, data: c.to_vec() }).encode()
        })
        .collect()
}

fn hash_of(m: &[&ModInfo]) -> Hash {
    mods_hash(m.iter().filter(|m| m.client_required).map(|m| (m.id.as_str(), m.version.as_str(), &m.sha256)))
}

/// A package padded to `n` chunks plus a bit, so transfers have several chunks.
fn big_package(id: &str, chunks: usize) -> Vec<u8> {
    let mut p = package(id, "1.0.0", "", "");
    // Pseudo-random bytes before the zip (readers find the central directory from the end).
    let mut seed = 1u32;
    let mut pre: Vec<u8> = (0..chunks * CHUNK_SIZE)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) as u8
        })
        .collect();
    pre.append(&mut p);
    pre
}

#[test]
fn vanilla_server_times_out() {
    let mut r = rig();
    assert_eq!(r.s.state_name(), "idle");
    let out = r.s.contact(0);
    let Message::Hello(h) = &sent(&out)[0] else { panic!() };
    assert_eq!((h.proto, h.platform.as_str()), (1, "android-armv7"));
    assert!(r.s.contact(5).is_empty());
    assert!(r.s.join_requested().is_empty());
    assert!(r.s.tick(1000).is_empty());
    assert_eq!(r.s.state_name(), "await-hello");
    let out = r.s.tick(1500);
    assert!(matches!(out[..], [Output::Join(None)]));
    assert_eq!(r.s.state_name(), "vanilla");
    assert!(r.s.tick(2000).is_empty());
    assert!(r.s.join_requested().is_empty());
    // late packets are ignored
    assert!(r.s.receive(&hello_pkt([0; 32], false, 0), 2000).is_empty());
    assert!(r.s.channel("demo:echo", b"x").is_none());
}

#[test]
fn join_before_contact_is_vanilla() {
    let mut r = rig();
    assert!(matches!(r.s.join_requested()[..], [Output::Join(None)]));
    assert!(r.s.contact(0).is_empty());
}

#[test]
fn modded_server_without_mods() {
    let mut r = rig();
    r.s.contact(0);
    let h = [9u8; 32];
    assert!(r.s.receive(&hello_pkt(h, false, 0), 10).is_empty());
    assert!(r.s.is_modded());
    assert_eq!(r.s.state_name(), "modded");
    assert!(matches!(r.s.join_requested()[..], [Output::Join(Some(x))] if x == h));
    // server-side ops and stale replies are ignored
    assert!(r.s.receive(&Message::ListRequest.encode(), 11).is_empty());
    assert!(r.s.receive(&list_pkt(vec![]), 11).is_empty());
}

#[test]
fn server_only_list_joins_without_prompt() {
    let mut r = rig();
    r.s.contact(0);
    assert!(r.s.join_requested().is_empty());
    // a CHANNEL before the join is dropped
    assert!(r.s.receive(&Message::Channel(Channel { channel_id: 3, payload: vec![] }).encode(), 1).is_empty());
    let out = r.s.receive(&hello_pkt([1; 32], false, 1), 10);
    assert!(matches!(sent(&out)[..], [Message::ListRequest]));
    assert_eq!(r.s.state_name(), "await-list");
    assert!(r.s.tick(20).is_empty());
    let out = r.s.receive(&list_pkt(vec![info("srv", b"x", false, &["server"])]), 20);
    assert!(matches!(out[..], [Output::Join(Some(_))]));
    // the channel announced in the list works now
    assert_eq!(r.s.channel("demo:echo", b"hi").unwrap(), [0xbc, 0x10, 3, 0, b'h', b'i']);
    assert!(r.s.channel("demo:nope", b"hi").is_none());
    assert!(r.s.channel("demo:echo", &vec![0; MAX_CHANNEL_PAYLOAD + 1]).is_none());
}

#[test]
fn list_timeout_and_garbage() {
    let mut r = rig();
    r.s.contact(0);
    assert!(r.s.receive(&[0xbc, 0x7f], 1).is_empty());
    assert!(r.s.receive(&[0xbc], 1).is_empty());
    r.s.receive(&hello_pkt([1; 32], true, 1), 10);
    let out = r.s.tick(10_010);
    assert!(matches!(&out[..], [Output::Failed(Failure::Protocol(_))]));
    assert_eq!(r.s.state_name(), "failed");
    // decide / loaded / list in the wrong state do nothing
    assert!(r.s.decide(true, 0).is_empty());
    assert!(r.s.loaded(Ok(vec![])).is_empty());
    assert!(r.s.receive(&list_pkt(vec![]), 0).is_empty());
}

#[test]
fn full_join_with_download_resume_and_channels() {
    let mut r = rig();
    let pkg = big_package("demo", 9);
    let cached_pkg = package("cachedmod", "1.0.0", "", "");
    r.cache.put(&cached_pkg).unwrap();
    let mut demo = info("demo", &pkg, true, &["server", "client"]);
    demo.id = "demo".into();
    let cm = info("cachedmod", &cached_pkg, true, &["client"]);
    let hash = hash_of(&[&demo, &cm]);
    // a partial file from an earlier, interrupted transfer (2.5 chunks)
    r.cache.write_part(&demo.sha256, 0, &pkg[..CHUNK_SIZE * 5 / 2]).unwrap();

    let out = r.s.contact(0);
    let Message::Hello(h) = &sent(&out)[0] else { panic!() };
    assert_eq!(h.cached, vec![cm.sha256]);
    r.s.receive(&hello_pkt(hash, true, 2), 10);
    let out = r.s.receive(&list_pkt(vec![cm.clone(), demo.clone()]), 20);
    let Output::Prompt { required, mods, download } = &out[0] else { panic!("{out:?}") };
    assert!(*required);
    assert_eq!(*download, demo.size);
    assert!(mods[0].cached && mods[0].client && !mods[1].cached);
    assert_eq!(r.s.state_name(), "prompt");
    assert!(r.s.join_requested().is_empty());

    let out = r.s.decide(true, 30);
    assert!(matches!(&out[0], Output::Progress { index: 0, count: 1, done, .. } if *done == 2 * CHUNK_SIZE as u64));
    assert!(
        matches!(&sent(&out)[..], [Message::FileRequest(FileRequest { offset, .. })] if *offset == 2 * CHUNK_SIZE as u64)
    );
    assert_eq!(r.s.state_name(), "download");
    // a chunk of another file is ignored, a FILE_ERROR for another file too
    let other = chunks(&cached_pkg, cm.sha256, 0);
    assert!(r.s.receive(&other[0], 31).is_empty());
    let fe = Message::FileError(FileError { sha256: cm.sha256, code: 1 }).encode();
    assert!(r.s.receive(&fe, 31).is_empty());
    // BUSY: wait, then ask again at the same offset
    let busy = Message::FileError(FileError { sha256: demo.sha256, code: file_error::BUSY }).encode();
    assert!(r.s.receive(&busy, 40).is_empty());
    assert!(matches!(&sent(&r.s.tick(140))[..], [Message::FileRequest(_)]));
    let mut acks = 0;
    let mut all = Vec::new();
    for c in chunks(&pkg, demo.sha256, 2) {
        let out = r.s.receive(&c, 200);
        acks += sent(&out).iter().filter(|m| matches!(m, Message::FileRequest(_))).count();
        all.extend(out);
    }
    assert!(acks >= 2, "{acks}");
    let Some(Output::Load(ok)) = all.iter().find(|o| matches!(o, Output::Load(_))) else { panic!("{all:?}") };
    assert_eq!(ok.iter().map(|a| a.info.id.as_str()).collect::<Vec<_>>(), ["cachedmod", "demo"]);
    assert_eq!(ok[1].manifest.id, "demo");
    assert_eq!(r.s.state_name(), "loading");
    assert!(r.s.cache.get(&demo.sha256).is_some());

    let out = r.s.loaded(Ok(vec![("demo".into(), "1.0.0".into())]));
    let m = sent(&out);
    assert!(matches!(&m[0], Message::Ready(Ready { accepted_hash, .. }) if *accepted_hash == hash));
    assert!(matches!(out.last(), Some(Output::Join(Some(x))) if *x == hash));
    assert!(r.s.is_modded());

    // channels: by list id, by a later CHANNEL_REGISTER, unknown ids dropped
    let ch = Message::Channel(Channel { channel_id: 3, payload: b"echo:hi".to_vec() }).encode();
    assert!(
        matches!(&r.s.receive(&ch, 300)[..], [Output::Channel { name, data }] if name == "demo:echo" && data == b"echo:hi")
    );
    let reg = Message::ChannelRegister(ChannelRegister { name: "demo:late".into(), id: 9 }).encode();
    assert!(r.s.receive(&reg, 300).is_empty());
    assert!(r.s.channel("demo:late", b"").is_some());
    let unk = Message::Channel(Channel { channel_id: 77, payload: vec![] }).encode();
    assert!(r.s.receive(&unk, 300).is_empty());
    let rt = Message::RejectText(RejectText { reason_code: 3, text: "no".into(), url: None }).encode();
    assert!(matches!(&r.s.receive(&rt, 300)[..], [Output::Reject(_)]));
}

#[test]
fn busy_retry_and_stall() {
    let mut r = rig();
    let pkg = package("demo", "1.0.0", "", "");
    let demo = info("demo", &pkg, true, &["client"]);
    r.s.contact(0);
    r.s.receive(&hello_pkt([2; 32], true, 1), 0);
    r.s.receive(&list_pkt(vec![demo.clone()]), 0);
    r.s.decide(true, 0);
    let busy = Message::FileError(FileError { sha256: demo.sha256, code: file_error::BUSY }).encode();
    r.s.receive(&busy, 10);
    assert!(r.s.tick(50).is_empty());
    let out = r.s.tick(110);
    assert!(matches!(&sent(&out)[..], [Message::FileRequest(FileRequest { offset: 0, .. })]));
    assert!(r.s.tick(20_000).is_empty());
    let out = r.s.tick(30_111);
    assert!(matches!(&out[..], [Output::Failed(Failure::Stalled(id))] if id == "demo"));
}

#[test]
fn download_failures() {
    let pkg = package("demo", "1.0.0", "", "");
    let start = |demo: &ModInfo| {
        let mut r = rig();
        r.s.contact(0);
        r.s.receive(&hello_pkt([2; 32], true, 1), 0);
        r.s.receive(&list_pkt(vec![demo.clone()]), 0);
        r.s.decide(true, 0);
        r
    };
    // FILE_ERROR NOT_FOUND
    let demo = info("demo", &pkg, true, &["client"]);
    let mut r = start(&demo);
    let fe = Message::FileError(FileError { sha256: demo.sha256, code: 1 }).encode();
    assert!(matches!(&r.s.receive(&fe, 1)[..], [Output::Failed(Failure::Download { code: 1, .. })]));
    // wrong total
    let mut r = start(&demo);
    let bad = Message::FileChunk(FileChunk { sha256: demo.sha256, offset: 0, total: demo.size + 1, data: pkg.clone() })
        .encode();
    assert!(matches!(&r.s.receive(&bad, 1)[..], [Output::Failed(Failure::Protocol(_))]));
    // short chunk that is not the last
    let mut r = start(&demo);
    let bad =
        Message::FileChunk(FileChunk { sha256: demo.sha256, offset: 0, total: demo.size + 10, data: vec![1] }).encode();
    assert!(matches!(&r.s.receive(&bad, 1)[..], [Output::Failed(Failure::Protocol(_))]));
    // hash mismatch: the listed hash belongs to other bytes of the same size
    let mut fake = demo.clone();
    fake.sha256 = [5; 32];
    let mut r = start(&fake);
    let c =
        Message::FileChunk(FileChunk { sha256: fake.sha256, offset: 0, total: fake.size, data: pkg.clone() }).encode();
    assert!(matches!(&r.s.receive(&c, 1).last(), Some(Output::Failed(Failure::HashMismatch(_)))));
    // disk errors: the cache dir is a file
    let mut r = start(&demo);
    std::fs::write(r.cache.dir().parent().unwrap().join("cache"), b"f").ok();
    let _ = std::fs::remove_dir_all(r.cache.dir());
    std::fs::write(r.cache.dir(), b"f").unwrap();
    let c =
        Message::FileChunk(FileChunk { sha256: demo.sha256, offset: 0, total: demo.size, data: pkg.clone() }).encode();
    assert!(matches!(&r.s.receive(&c, 1)[..], [Output::Failed(Failure::Disk(_))]));
}

#[test]
fn finish_disk_error() {
    let pkg = package("demo", "1.0.0", "", "");
    let demo = info("demo", &pkg, true, &["client"]);
    let mut r = rig();
    r.s.contact(0);
    r.s.receive(&hello_pkt([2; 32], true, 1), 0);
    r.s.receive(&list_pkt(vec![demo.clone()]), 0);
    r.s.decide(true, 0);
    // the target name is taken by a directory, so the rename fails
    std::fs::create_dir_all(r.cache.dir().join(format!("{}.bhmod", bhmod_proto::hex(&demo.sha256))).join("x")).unwrap();
    let c = Message::FileChunk(FileChunk { sha256: demo.sha256, offset: 0, total: demo.size, data: pkg }).encode();
    assert!(matches!(&r.s.receive(&c, 1).last(), Some(Output::Failed(Failure::Disk(_)))));
}

#[test]
fn refusals_decline_and_load_failure() {
    let ok = package("demo", "1.0.0", "", "");
    let wasm = package("wasmmod", "1.0.0", "", "wasm");
    let cache_ready = |mods: Vec<ModInfo>, required: bool| {
        let mut r = rig();
        r.cache.put(&ok).unwrap();
        r.cache.put(&wasm).unwrap();
        r.s.contact(0);
        r.s.receive(&hello_pkt([4; 32], required, mods.len() as u64), 0);
        r.s.receive(&list_pkt(mods), 0);
        r
    };
    let demo = info("demo", &ok, true, &["client"]);
    // declined, required
    let mut r = cache_ready(vec![demo.clone()], true);
    assert!(matches!(&r.s.decide(false, 0)[..], [Output::Failed(Failure::Declined)]));
    // declined, optional only
    let mut opt = demo.clone();
    opt.client_required = false;
    let mut r = cache_ready(vec![opt.clone()], false);
    r.s.join_requested();
    assert!(matches!(&r.s.decide(false, 0)[..], [Output::Join(Some(_))]));
    // native is refused before any download
    let mut nat = demo.clone();
    nat.permissions.native = true;
    let mut r = cache_ready(vec![nat], true);
    assert!(matches!(&r.s.decide(true, 0)[..], [Output::Failed(Failure::Refused { refusal: Refusal::Native, .. })]));
    // a required mod that this loader cannot run fails; an optional one is skipped
    let w = info("wasmmod", &wasm, true, &["client"]);
    let mut r = cache_ready(vec![demo.clone(), w.clone()], true);
    assert!(matches!(&r.s.decide(true, 0)[..], [Output::Failed(Failure::Refused { .. })]));
    let mut wo = w.clone();
    wo.client_required = false;
    let mut r = cache_ready(vec![demo.clone(), wo], true);
    let out = r.s.decide(true, 0);
    assert!(matches!(&out[..], [Output::Load(v)] if v.len() == 1));
    assert!(matches!(&r.s.loaded(Err("boom".into()))[..], [Output::Failed(Failure::LoadFailed(_))]));
    // a cached package that cannot be read
    let mut r = cache_ready(vec![demo.clone()], true);
    let p = r.cache.get(&demo.sha256).unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let out = r.s.decide(true, 0);
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    assert!(matches!(&out[..], [Output::Failed(Failure::Disk(_))]), "{out:?}");
}

#[test]
fn reject_text_before_server_hello() {
    let mut r = rig();
    r.s.contact(0);
    r.s.join_requested();
    let rt = Message::RejectText(RejectText { reason_code: 3, text: "update".into(), url: None }).encode();
    let out = r.s.receive(&rt, 1);
    assert!(matches!(&out[..], [Output::Reject(_), Output::Join(None)]));
}

#[test]
fn failure_texts() {
    for f in [
        Failure::Declined,
        Failure::Refused { id: "a".into(), refusal: Refusal::Native },
        Failure::Download { id: "a".into(), code: 1 },
        Failure::Stalled("a".into()),
        Failure::HashMismatch("a".into()),
        Failure::Protocol("x".into()),
        Failure::Disk("x".into()),
        Failure::LoadFailed("x".into()),
    ] {
        assert!(!f.to_string().is_empty() && !f.code().is_empty());
    }
}
