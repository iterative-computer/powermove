//! Research proof ONLY. This file is test-only, never linked into the library or
//! CLI. All passwords and ciphertexts are synthetic; no Security.framework call.
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Result, ensure};
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;

enum Platform {
    Mac,
    Linux,
}
fn decrypt(platform: Platform, password: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.is_empty() {
        return Ok(Vec::new());
    }
    ensure!(ciphertext.len() >= 3, "short synthetic ciphertext");
    let iterations = match (platform, &ciphertext[..3]) {
        (Platform::Mac, b"v10") => 1003,
        (Platform::Linux, b"v11") => 1,
        _ => anyhow::bail!("unsupported platform/version pair"),
    };
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password, b"saltysalt", iterations, &mut key);
    let result = cbc::Decryptor::<aes::Aes128>::new(&key.into(), &[b' '; 16].into())
        .decrypt_padded_vec_mut::<Pkcs7>(&ciphertext[3..])
        .map_err(|_| anyhow::anyhow!("invalid synthetic padding"));
    key.fill(0);
    result
}
#[test]
fn synthetic_mac_v10_openssl_vector_and_linux_v11_roundtrip() {
    // Independently produced by Python hashlib.pbkdf2_hmac + openssl enc.
    let password = b"S6 synthetic password";
    let plaintext = b"S6 synthetic secret; never a user credential";
    let vector=hex::decode("763130e76d41648b2b30948031bf9143544a95602e7917cb34fa8a573dd86ef951d70c835be494f8085c4eddf66e2b9dfe9d1d").unwrap();
    assert_eq!(
        decrypt(Platform::Mac, password, &vector).unwrap(),
        plaintext
    );
    assert!(decrypt(Platform::Mac, password, b"v11not-a-mac-format").is_err());
    let mut key = [0; 16];
    pbkdf2_hmac::<Sha1>(password, b"saltysalt", 1, &mut key);
    let mut linux = b"v11".to_vec();
    linux.extend(
        cbc::Encryptor::<aes::Aes128>::new(&key.into(), &[b' '; 16].into())
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext),
    );
    assert_eq!(
        decrypt(Platform::Linux, password, &linux).unwrap(),
        plaintext
    );
    assert!(decrypt(Platform::Mac, b"wrong synthetic password", &vector).is_err());
    assert!(decrypt(Platform::Mac, password, &vector[..vector.len() - 1]).is_err());
}
