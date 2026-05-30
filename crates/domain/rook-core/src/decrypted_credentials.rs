// DecryptedCredentials — domain representation of decrypted provider credentials
//
// After encryption boundary is crossed, credentials exist in plaintext form
// as this enum for use by provider constructors.

use serde::{Deserialize, Serialize};

/// Plaintext credentials after decryption from [`super::Credentials`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DecryptedCredentials {
    ApiKey {
        api_key: String,
    },
    OAuth {
        email: String,
        access_token: String,
        refresh_token: String,
        expires_at: i64,
        scope: String,
        id_token: String,
        project_id: String,
    },
}
