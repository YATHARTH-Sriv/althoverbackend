use sha2::{Digest, Sha256};

pub fn anchor_discriminator(value: &str) -> [u8; 8] {
    let digest = Sha256::digest(value.as_bytes());
    let mut discriminator = [0u8; 8];
    discriminator.copy_from_slice(&digest[..8]);
    discriminator
}
