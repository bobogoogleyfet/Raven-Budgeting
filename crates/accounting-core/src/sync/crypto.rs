//! Encrypting a bundle where it is written.
//!
//! # Why this is not optional
//!
//! A bundle is a household's whole ledger — every shop, every amount, every
//! date — and it travels by being left somewhere the other device can reach:
//! a synced folder, a stick, a phone's downloads. Every one of those is
//! readable by something other than the two devices involved, and a sync file
//! sitting in Dropbox in the clear is a bank statement in a shared drawer. The
//! transport is chosen for convenience, so the file has to defend itself.
//!
//! # The choices
//!
//! **ChaCha20-Poly1305** rather than AES-GCM. It is constant-time in software
//! without hardware support — a phone may or may not expose AES instructions
//! to us, and the failure mode of not having them is a timing side channel
//! rather than an error anybody would notice.
//!
//! **Argon2id** to turn a phrase into a key. The phrase has to be something a
//! person can read to somebody else across a kitchen, which means it has far
//! less entropy than a key; a plain hash would let anyone holding the file try
//! a billion guesses a second. Argon2id makes each guess cost memory as well
//! as time, which is what closes the gap between "a phrase two people can
//! remember" and "a key".
//!
//! **A fresh salt and nonce per file**, both stored in the header. Reusing a
//! nonce with the same key is the one catastrophic mistake available here — it
//! leaks the XOR of two plaintexts — and deriving them from anything stable
//! would guarantee it. They are not secret; they only have to be unique.

use argon2::Argon2;
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::error::{CoreError, Result};

/// Marks the file as ours, so a wrong file chosen in a picker fails with
/// "this is not a sync file" rather than "the phrase is wrong" — which would
/// send somebody hunting for a typo in a phrase that was never the problem.
const MAGIC: &[u8; 8] = b"PACCSYNC";

/// The envelope's version, separate from the bundle's own.
///
/// A change to how the file is encrypted is not a change to what is inside it,
/// and conflating the two would mean a crypto upgrade looking like a data
/// format break to every device that had not updated yet.
const ENVELOPE: u8 = 1;

const SALT: usize = 16;
const NONCE: usize = 12;
const KEY: usize = 32;

/// Derive the file key from a pairing phrase.
///
/// Deliberately slow. The cost is paid once per file, and it is the whole
/// defence for a secret short enough to say out loud.
fn key_from(phrase: &str, salt: &[u8]) -> Result<[u8; KEY]> {
    let mut key = [0u8; KEY];
    Argon2::default()
        .hash_password_into(phrase.as_bytes(), salt, &mut key)
        .map_err(|e| CoreError::Invalid(format!("could not derive a key: {e}")))?;
    Ok(key)
}

/// Wrap a bundle for the journey.
///
/// Layout: `MAGIC | version | salt | nonce | ciphertext+tag`. The header is
/// plaintext because the receiver needs it to derive the key at all, and it is
/// authenticated as associated data so that a file whose salt has been swapped
/// fails as tampering rather than as a wrong phrase.
pub fn seal(plaintext: &[u8], phrase: &str) -> Result<Vec<u8>> {
    if phrase.is_empty() {
        return Err(CoreError::Invalid(
            "a sync file needs a pairing phrase; an empty one protects nothing".to_string(),
        ));
    }
    // The same source the nonce comes from, rather than a second randomness
    // dependency: on Android and iOS every extra crate that reaches for
    // entropy is another one that has to be right about how to ask.
    let mut salt = [0u8; SALT];
    OsRng.fill_bytes(&mut salt);

    let key = key_from(phrase, &salt)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

    let mut header = Vec::with_capacity(MAGIC.len() + 1 + SALT + NONCE);
    header.extend_from_slice(MAGIC);
    header.push(ENVELOPE);
    header.extend_from_slice(&salt);
    header.extend_from_slice(nonce.as_slice());

    let sealed = cipher
        .encrypt(
            &nonce,
            chacha20poly1305::aead::Payload {
                msg: plaintext,
                aad: &header,
            },
        )
        .map_err(|_| CoreError::Invalid("could not encrypt the sync file".to_string()))?;

    let mut out = header;
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Unwrap a bundle.
///
/// A wrong phrase and a tampered file are indistinguishable here, and that is
/// correct: an authentication failure does not say which byte was wrong, and
/// pretending to know would be a guessing oracle. The message says both are
/// possible rather than blaming the user for a typo they may not have made.
pub fn open(bytes: &[u8], phrase: &str) -> Result<Vec<u8>> {
    let head = MAGIC.len() + 1 + SALT + NONCE;
    if bytes.len() <= head || &bytes[..MAGIC.len()] != MAGIC {
        return Err(CoreError::Invalid("this is not a sync file".to_string()));
    }
    let version = bytes[MAGIC.len()];
    if version != ENVELOPE {
        return Err(CoreError::Invalid(format!(
            "this sync file uses envelope {version}, and this version of the app reads \
             {ENVELOPE}; update the app on this device"
        )));
    }

    let salt = &bytes[MAGIC.len() + 1..MAGIC.len() + 1 + SALT];
    let nonce = &bytes[MAGIC.len() + 1 + SALT..head];
    let key = key_from(phrase, salt)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));

    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            chacha20poly1305::aead::Payload {
                msg: &bytes[head..],
                aad: &bytes[..head],
            },
        )
        .map_err(|_| {
            CoreError::Invalid(
                "could not read this sync file: the pairing phrase does not match, or the \
                 file has been altered"
                    .to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASE: &str = "correct horse battery staple";

    #[test]
    fn a_sealed_bundle_comes_back_out() {
        let sealed = seal(b"{\"format\":1}", PHRASE).expect("seal");
        assert_eq!(open(&sealed, PHRASE).expect("open"), b"{\"format\":1}");
    }

    #[test]
    fn the_ledger_is_not_sitting_in_the_file() {
        // The point of the exercise. A bundle names every shop and every
        // amount, and it is left in a folder chosen for convenience.
        let sealed = seal(b"Leatherby's 13.59", PHRASE).expect("seal");
        assert!(
            !sealed.windows(11).any(|w| w == b"Leatherby's"),
            "the plaintext must not be in the file"
        );
    }

    #[test]
    fn the_wrong_phrase_does_not_open_it() {
        let sealed = seal(b"secret", PHRASE).expect("seal");
        let error = open(&sealed, "not the phrase").expect_err("should refuse");
        // Says both possibilities rather than blaming a typo the user may not
        // have made.
        assert!(error.to_string().contains("does not match"));
        assert!(error.to_string().contains("altered"));
    }

    #[test]
    fn a_tampered_byte_is_caught() {
        // Poly1305 is what makes this a modification, not a decryption that
        // quietly yields different numbers.
        let mut sealed = seal(b"subtotal 1359", PHRASE).expect("seal");
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(open(&sealed, PHRASE).is_err());
    }

    #[test]
    fn swapping_the_salt_is_tampering_rather_than_a_wrong_phrase() {
        // The header is plaintext because the receiver needs it, so it is
        // authenticated as associated data instead.
        let mut sealed = seal(b"secret", PHRASE).expect("seal");
        sealed[MAGIC.len() + 1] ^= 0xff;
        assert!(open(&sealed, PHRASE).is_err());
    }

    #[test]
    fn two_files_of_the_same_thing_share_no_nonce() {
        // Reusing a nonce with one key leaks the XOR of two plaintexts, and it
        // is the only catastrophic mistake available here.
        let a = seal(b"same", PHRASE).expect("seal");
        let b = seal(b"same", PHRASE).expect("seal");
        assert_ne!(a, b, "a fresh salt and nonce per file");
        let head = MAGIC.len() + 1 + SALT;
        assert_ne!(a[head..head + NONCE], b[head..head + NONCE]);
    }

    #[test]
    fn something_that_is_not_a_sync_file_says_so() {
        // A wrong file chosen in a picker must not read as a wrong phrase, or
        // somebody goes hunting for a typo that was never the problem.
        let error = open(b"this is a photo, honestly", PHRASE).expect_err("refuse");
        assert!(error.to_string().contains("not a sync file"));
    }

    #[test]
    fn an_empty_phrase_is_refused_rather_than_quietly_accepted() {
        assert!(seal(b"secret", "").is_err());
    }
}
