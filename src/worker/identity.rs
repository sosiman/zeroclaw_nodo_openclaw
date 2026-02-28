use ed25519_dalek::SigningKey;
use chacha20poly1305::aead::OsRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

#[derive(Clone)]
pub struct NodeIdentity {
    pub node_id: String,
    pub signing_key: SigningKey,
    pub created_at: i64,
    pub protocol_version: i32,
}

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    node_id: String,
    private_key_base64: String,
    created_at: i64,
    protocol_version: i32,
}

impl NodeIdentity {
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        
        if path.exists() {
            let data = fs::read_to_string(path)?;
            let stored: StoredIdentity = serde_json::from_str(&data)?;
            
            // Decode the private key
            let key_bytes = BASE64.decode(&stored.private_key_base64)?;
            let signing_key = SigningKey::from_bytes(key_bytes.as_slice().try_into().unwrap());
            
            let verifying_key = signing_key.verifying_key();
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(verifying_key.as_bytes());
            let node_id = hex::encode(hasher.finalize());
            
            Ok(Self {
                node_id,
                signing_key,
                created_at: stored.created_at,
                protocol_version: stored.protocol_version,
            })
        } else {
            // Create a new identity
            let mut csprng = OsRng;
            let signing_key = SigningKey::generate(&mut csprng);
            let verifying_key = signing_key.verifying_key();
            
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(verifying_key.as_bytes());
            let node_id = hex::encode(hasher.finalize());
            let created_at = chrono::Utc::now().timestamp_millis();
            let protocol_version = 3; // From protocol.md

            let identity = Self {
                node_id: node_id.clone(),
                signing_key: signing_key.clone(),
                created_at,
                protocol_version,
            };

            // Save to disk securely
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            let private_key_base64 = BASE64.encode(signing_key.as_bytes());
            let stored = StoredIdentity {
                node_id,
                private_key_base64,
                created_at,
                protocol_version,
            };

            let data = serde_json::to_string_pretty(&stored)?;
            fs::write(path, data)?;
            
            // Enforce 0600 permissions on Unix (dummy implementation for Windows compat during dev)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(path)?.permissions();
                perms.set_mode(0o600);
                fs::set_permissions(path, perms)?;
            }

            Ok(identity)
        }
    }

    pub fn public_key_base64(&self) -> String {
        let verifying_key = self.signing_key.verifying_key();
        BASE64.encode(verifying_key.as_bytes())
    }

    pub fn sign_nonce(&self, nonce: &str) -> String {
        use ed25519_dalek::Signer;
        let signature = self.signing_key.sign(nonce.as_bytes());
        BASE64.encode(signature.to_bytes())
    }

    pub fn sign_payload(&self, payload: &[u8]) -> String {
        use ed25519_dalek::Signer;
        let signature = self.signing_key.sign(payload);
        BASE64.encode(signature.to_bytes())
    }

    pub fn rotate_key<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        let mut csprng = OsRng;
        let new_signing_key = SigningKey::generate(&mut csprng);
        
        // We do NOT change the node_id during key rotation to preserve identity continuity
        let private_key_base64 = BASE64.encode(new_signing_key.as_bytes());
        let stored = StoredIdentity {
            node_id: self.node_id.clone(),
            private_key_base64,
            created_at: self.created_at,
            protocol_version: self.protocol_version,
        };

        let data = serde_json::to_string_pretty(&stored)?;
        fs::write(path.as_ref(), data)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(mut perms) = fs::metadata(path.as_ref()).map(|m| m.permissions()) {
                perms.set_mode(0o600);
                let _ = fs::set_permissions(path.as_ref(), perms);
            }
        }

        self.signing_key = new_signing_key;
        Ok(())
    }
}
