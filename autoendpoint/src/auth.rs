use openssl::error::ErrorStack;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;

/// Sign some data with a key and return the hex representation
pub fn sign_with_key(key: &[u8], data: &[u8]) -> Result<String, ErrorStack> {
    let key = PKey::hmac(key)?;
    let mut signer = Signer::new(MessageDigest::sha256(), &key)?;

    signer.update(data)?;
    Ok(hex::encode(signer.sign_to_vec()?))
}
