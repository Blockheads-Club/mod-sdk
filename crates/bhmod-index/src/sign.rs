//! minisign (ed25519) signatures for `index.json`.

use std::io::Cursor;

use minisign::{KeyPair, PError, PublicKey, PublicKeyBox, SecretKeyBox, SignatureBox};

/// A new unencrypted key pair as `(public key file, secret key file)` text, in minisign format.
/// Store the secret key in the `INDEX_SIGNING_KEY` repo secret; ship the public key with the consumers.
pub fn keygen() -> (String, String) {
    let kp = KeyPair::generate_unencrypted_keypair().expect("system random generator works");
    let pk = kp.pk.to_box().expect("a public key encodes").to_string();
    let sk = kp.sk.to_box(Some("Blockheads mod index signing key")).expect("a fresh key encodes").to_string();
    (pk, sk)
}

fn bad_secret(e: PError) -> String {
    format!("bad secret key: {e}")
}

fn bad_public(e: PError) -> String {
    format!("bad public key: {e}")
}

fn bad_signature(e: PError) -> String {
    format!("bad signature: {e}")
}

/// Signs `data` with a minisign secret key file's text; `password` for encrypted keys (`minisign -G`).
/// Returns the `.minisig` file text.
pub fn sign(data: &[u8], secret_key: &str, password: Option<String>, trusted_comment: &str) -> Result<String, String> {
    let sk_box = SecretKeyBox::from_string(secret_key.trim()).map_err(bad_secret)?;
    let sk = match password {
        Some(p) => sk_box.into_secret_key(Some(p)),
        None => sk_box.into_unencrypted_secret_key(),
    }
    .map_err(bad_secret)?;
    let sig =
        minisign::sign(None, &sk, Cursor::new(data), Some(trusted_comment), Some("Blockheads mod index signature"))
            .expect("a decrypted key signs");
    Ok(sig.to_string())
}

/// Reads a public key: a minisign `.pub` file or its bare base64 line.
pub fn public_key(text: &str) -> Result<PublicKey, String> {
    let text = text.trim();
    let parsed = if text.starts_with("untrusted comment:") {
        PublicKeyBox::from_string(text).and_then(PublicKeyBox::into_public_key)
    } else {
        PublicKey::from_base64(text)
    };
    parsed.map_err(bad_public)
}

/// Verifies a `.minisig` signature over `data`.
pub fn verify(data: &[u8], public_key_text: &str, signature: &str) -> Result<(), String> {
    let pk = public_key(public_key_text)?;
    let sig = SignatureBox::from_string(signature).map_err(bad_signature)?;
    minisign::verify(&pk, &sig, Cursor::new(data), true, false, false).map_err(bad_signature)
}
