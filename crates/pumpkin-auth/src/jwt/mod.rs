//! # JWT Verifier for Minecraft: Bedrock Edition
//!
//! This module provides the core functionality for verifying the chain of JWT tokens
//! sent by a Minecraft: Bedrock Edition client. It handles cryptographic signature
//! verification, public key extraction, and decoding of player data.

use base64::{Engine as _, engine::general_purpose};
use ecdsa::Signature;
use p384::PublicKey;
use p384::ecdsa::{VerifyingKey, signature::Verifier};
use p384::pkcs8::DecodePublicKey;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// Represents the claims extracted from a Minecraft Bedrock player's JWT token.
///
/// This struct contains the player's display name, UUID, and XUID.
#[derive(Debug)]
pub struct PlayerClaims {
    /// The player's display name (in-game name).
    pub display_name: String,
    /// The player's unique identifier (UUID).
    pub uuid: String,
    /// The player's Xbox User ID (XUID).
    pub xuid: String,
}

/// Represents the possible errors that can occur during JWT verification.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Indicates that a JWT token has an invalid format (not enough parts).
    #[error("Invalid token format")]
    InvalidTokenFormat,
    /// Indicates that the 'x5u' (X.509 URL) header parameter is missing from a token.
    #[error("x5u not found in header")]
    MissingX5U,
    /// Indicates a failure in Base64 decoding.
    #[error("Base64 decoding failed: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    /// Indicates a failure in parsing JSON data.
    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
    /// Indicates a failure in building a public key from its representation.
    #[error("Public key build failed: {0}")]
    PublicKeyBuild(String),
    /// Indicates that the token was not signed by the trusted Mojang public key.
    #[error("Token not signed by trusted Mojang key")]
    MojangKeyMismatch,
    /// Indicates that the token's signature is invalid.
    #[error("Invalid signature")]
    InvalidSignature,
    /// Indicates an error related to ECDSA signature operations.
    #[error("ECDSA signature error: {0}")]
    Ecdsa(#[from] ecdsa::Error),
}

/// Decodes a Base64 URL-safe encoded string with no padding.
///
/// # Arguments
///
/// * `s` - The Base64 URL-safe encoded string to decode.
///
/// # Returns
///
/// A `Result` containing the decoded bytes or a `base64::DecodeError`.
pub fn decode_b64_url_nopad(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    general_purpose::URL_SAFE_NO_PAD.decode(s)
}

/// Decodes a standard Base64 encoded string.
///
/// # Arguments
///
/// * `s` - The standard Base64 encoded string to decode.
///
/// # Returns
///
/// A `Result` containing the decoded bytes or a `base64::DecodeError`.
pub fn decode_b64_standard(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    general_purpose::STANDARD.decode(s)
}

/// Builds a P-384 public key from a Base64 encoded string.
///
/// This function supports several common public key formats.
///
/// # Arguments
///
/// * `b64` - The Base64 encoded public key.
///
/// # Returns
///
/// A `Result` containing the `p384::PublicKey` or an `AuthError`.
pub fn build_public_key_from_b64(b64: &str) -> Result<PublicKey, AuthError> {
    let bytes = decode_b64_standard(b64)
        .or_else(|_| general_purpose::URL_SAFE.decode(b64))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(b64))
        .map_err(AuthError::Base64Decode)?;

    if !bytes.is_empty() && bytes[0] == 0x30 {
        PublicKey::from_public_key_der(&bytes).map_err(|e| AuthError::PublicKeyBuild(e.to_string()))
    } else if bytes.len() == 97 && bytes[0] == 0x04 {
        PublicKey::from_sec1_bytes(&bytes).map_err(|e| AuthError::PublicKeyBuild(e.to_string()))
    } else if bytes.len() == 96 {
        let mut sec1 = Vec::with_capacity(97);
        sec1.push(0x04u8);
        sec1.extend_from_slice(&bytes);
        PublicKey::from_sec1_bytes(&sec1).map_err(|e| AuthError::PublicKeyBuild(e.to_string()))
    } else {
        Err(AuthError::PublicKeyBuild(format!(
            "Unsupported key format/length: {} bytes",
            bytes.len()
        )))
    }
}

/// Decodes the header of a JWT and extracts the 'x5u' (X.509 URL) value.
///
/// # Arguments
///
/// * `header_b64` - The Base64 URL-safe encoded header of the JWT.
///
/// # Returns
///
/// A `Result` containing the 'x5u' value as a string or an `AuthError`.
pub fn decode_header_get_x5u(header_b64: &str) -> Result<String, AuthError> {
    let header_bytes = decode_b64_url_nopad(header_b64)?;
    let header_json: Value = serde_json::from_slice(&header_bytes)?;
    if let Some(x5u) = header_json.get("x5u")
        && let Some(s) = x5u.as_str()
    {
        return Ok(s.to_string());
    }
    Err(AuthError::MissingX5U)
}

/// JSON Web Key Set containing a vector of JWK public keys.
#[derive(Debug, Deserialize, Clone)]
pub struct Jwks {
    /// Vector of public keys.
    pub keys: Vec<Jwk>,
}

/// JSON Web Key representing an EC or RSA public key.
#[derive(Debug, Deserialize, Clone)]
pub struct Jwk {
    /// Key type (`"EC"` or `"RSA"`).
    pub kty: String,
    /// Optional algorithm specifier.
    #[serde(default)]
    pub alg: Option<String>,
    /// Optional curve type (e.g. `"P-384"`).
    #[serde(default)]
    pub crv: Option<String>,
    /// Optional X coordinate for EC key.
    pub x: Option<String>,
    /// Optional Y coordinate for EC key.
    pub y: Option<String>,
    /// Optional RSA modulus.
    pub n: Option<String>,
    /// Optional RSA exponent.
    pub e: Option<String>,
    /// Key ID matching the JWT header `kid`.
    pub kid: String,
}

impl Jwk {
    /// Converts this JWK into an Elliptic Curve `PublicKey`.
    pub fn to_ec_public_key(&self) -> Result<PublicKey, AuthError> {
        if self.kty != "EC" {
            return Err(AuthError::PublicKeyBuild(format!(
                "Unsupported JWK kty for EC: {}",
                self.kty
            )));
        }

        if let Some(ref crv) = self.crv
            && crv != "P-384"
        {
            return Err(AuthError::PublicKeyBuild(format!(
                "Unsupported JWK crv: {crv}"
            )));
        }

        let x = self.x.as_ref().ok_or_else(|| {
            AuthError::PublicKeyBuild("JWK missing x coordinate for EC key".into())
        })?;
        let y = self.y.as_ref().ok_or_else(|| {
            AuthError::PublicKeyBuild("JWK missing y coordinate for EC key".into())
        })?;

        let x_bytes = decode_b64_url_nopad(x)?;
        let y_bytes = decode_b64_url_nopad(y)?;

        if x_bytes.len() != 48 || y_bytes.len() != 48 {
            return Err(AuthError::PublicKeyBuild(
                "Invalid P-384 coordinate lengths".into(),
            ));
        }

        let mut sec1 = Vec::with_capacity(97);
        sec1.push(0x04u8);
        sec1.extend_from_slice(&x_bytes);
        sec1.extend_from_slice(&y_bytes);

        PublicKey::from_sec1_bytes(&sec1).map_err(|e| AuthError::PublicKeyBuild(e.to_string()))
    }

    /// Converts this JWK into an RSA `RsaPublicKey`.
    pub fn to_rsa_public_key(&self) -> Result<rsa::RsaPublicKey, AuthError> {
        if self.kty != "RSA" {
            return Err(AuthError::PublicKeyBuild(format!(
                "Unsupported JWK kty for RSA: {}",
                self.kty
            )));
        }

        let n = self
            .n
            .as_ref()
            .ok_or_else(|| AuthError::PublicKeyBuild("JWK missing n modulus for RSA key".into()))?;
        let e = self.e.as_ref().ok_or_else(|| {
            AuthError::PublicKeyBuild("JWK missing e exponent for RSA key".into())
        })?;

        let n_bytes = decode_b64_url_nopad(n)?;
        let e_bytes = decode_b64_url_nopad(e)?;

        let n_boxed = crypto_bigint::BoxedUint::from_be_slice_vartime(&n_bytes);
        let e_boxed = crypto_bigint::BoxedUint::from_be_slice_vartime(&e_bytes);

        rsa::RsaPublicKey::new(n_boxed, e_boxed)
            .map_err(|err| AuthError::PublicKeyBuild(err.to_string()))
    }
}

/// Default expected issuer for Minecraft OIDC authentication tokens.
pub const OIDC_ISSUER: &str = "https://identity.minecraft-services.net";
/// Default expected audience for Minecraft OIDC authentication tokens.
pub const OIDC_AUDIENCE: &str = "api://auth-minecraft-services/multiplayer";
/// Discovery endpoint URL for Minecraft Bedrock OIDC JWKS keys.
pub const OIDC_DISCOVERY_URL: &str =
    "https://client.discovery.minecraft-services.net/api/v1.0/discovery/MinecraftPE/builds/1.0.0.0";

/// Fetches the OIDC JSON Web Key Set (JWKS) from the discovery endpoint.
pub async fn fetch_oidc_jwks(
    discovery_url: Option<&str>,
    connect_timeout_ms: u32,
    read_timeout_ms: u32,
) -> Result<(String, Jwks), AuthError> {
    let url = discovery_url.unwrap_or(OIDC_DISCOVERY_URL);
    let client = crate::client_builder()
        .connect_timeout(std::time::Duration::from_millis(connect_timeout_ms as u64))
        .timeout(std::time::Duration::from_millis(read_timeout_ms as u64))
        .build()
        .map_err(|e| AuthError::PublicKeyBuild(e.to_string()))?;

    let discovery: Value = client
        .get(url)
        .send()
        .await
        .map_err(|e| AuthError::PublicKeyBuild(e.to_string()))?
        .json()
        .await
        .map_err(|e| AuthError::PublicKeyBuild(e.to_string()))?;

    let service_uri = discovery
        .get("result")
        .and_then(|v| v.get("serviceEnvironments"))
        .and_then(|v| v.get("auth"))
        .and_then(|v| v.get("prod"))
        .and_then(|v| v.get("serviceUri"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::PublicKeyBuild("Discovery missing serviceUri".into()))?;

    let openid_config_url = format!("{service_uri}/.well-known/openid-configuration");
    let openid_config: Value = client
        .get(&openid_config_url)
        .send()
        .await
        .map_err(|e| AuthError::PublicKeyBuild(e.to_string()))?
        .json()
        .await
        .map_err(|e| AuthError::PublicKeyBuild(e.to_string()))?;

    let jwks_uri = openid_config
        .get("jwks_uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::PublicKeyBuild("OpenID config missing jwks_uri".into()))?;

    let issuer = openid_config
        .get("issuer")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::PublicKeyBuild("OpenID config missing issuer".into()))?
        .to_string();

    let jwks: Jwks = client
        .get(jwks_uri)
        .send()
        .await
        .map_err(|e| AuthError::PublicKeyBuild(e.to_string()))?
        .json()
        .await
        .map_err(|e| AuthError::PublicKeyBuild(e.to_string()))?;

    Ok((issuer, jwks))
}

/// Verifies an OIDC token against a provided JWKS key set and expected issuer.
pub fn verify_oidc_token(
    token: &str,
    expected_issuer: &str,
    jwks: &Jwks,
) -> Result<PlayerClaims, AuthError> {
    let mut parts = token.split('.');
    let header_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let payload_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let signature_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;

    let signing_input = format!("{header_b64}.{payload_b64}");

    let header_bytes = decode_b64_url_nopad(header_b64)?;
    let header: Value = serde_json::from_slice(&header_bytes)?;
    let kid = header
        .get("kid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::PublicKeyBuild("OIDC header missing kid".into()))?;
    let alg = header
        .get("alg")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::PublicKeyBuild("OIDC header missing alg".into()))?;

    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.kid == kid)
        .ok_or_else(|| AuthError::PublicKeyBuild(format!("Key not found in JWKS: {kid}")))?;

    if alg == "ES384" {
        verify_es384_signature(&jwk.to_ec_public_key()?, &signing_input, signature_b64)?;
    } else if alg == "RS256" {
        verify_rs256_signature(jwk, &signing_input, signature_b64)?;
    } else {
        return Err(AuthError::PublicKeyBuild(format!(
            "Unsupported OIDC algorithm: {alg}"
        )));
    }

    let payload_bytes = decode_b64_url_nopad(payload_b64)?;
    let v: Value = serde_json::from_slice(&payload_bytes)?;

    verify_oidc_claims(&v, Some(expected_issuer))?;

    Ok(extract_oidc_player_claims(&v))
}

/// Verifies a self-signed OIDC token.
pub fn verify_oidc_token_self_signed(token: &str) -> Result<PlayerClaims, AuthError> {
    let mut parts = token.split('.');
    let header_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let payload_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let signature_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;

    let signing_input = format!("{header_b64}.{payload_b64}");

    let header_bytes = decode_b64_url_nopad(header_b64)?;
    let header: Value = serde_json::from_slice(&header_bytes)?;
    let alg = header
        .get("alg")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::PublicKeyBuild("OIDC header missing alg".into()))?;
    if alg == "ES384" {
        verify_es384_signature(
            &PublicKey::from_public_key_der(&decode_b64_standard(&decode_header_get_x5u(
                header_b64,
            )?)?)
            .map_err(|_| AuthError::PublicKeyBuild("Couldn't build public key from x5u".into()))?,
            &signing_input,
            signature_b64,
        )?;
    } else {
        return Err(AuthError::PublicKeyBuild(format!(
            "Unsupported OIDC algorithm (for self-signed): {alg}"
        )));
    }

    let payload_bytes = decode_b64_url_nopad(payload_b64)?;
    let v: Value = serde_json::from_slice(&payload_bytes)?;

    verify_oidc_claims(&v, None)?;

    Ok(untrusted_player_claims(&v))
}

fn verify_es384_signature(
    public_key: &PublicKey,
    signing_input: &str,
    signature_b64: &str,
) -> Result<(), AuthError> {
    let verifying_key = VerifyingKey::from(public_key);

    let sig_bytes = decode_b64_url_nopad(signature_b64)?;
    let signature = Signature::from_slice(&sig_bytes).map_err(|_| AuthError::InvalidSignature)?;

    verifying_key
        .verify(signing_input.as_bytes(), &signature)
        .map_err(|_| AuthError::InvalidSignature)
}

fn verify_rs256_signature(
    jwk: &Jwk,
    signing_input: &str,
    signature_b64: &str,
) -> Result<(), AuthError> {
    use rsa::pkcs1v15::VerifyingKey as RsaVerifyingKey;
    use rsa::signature::Verifier;
    use sha2::Sha256;

    let public_key = jwk.to_rsa_public_key()?;
    let sig_bytes = decode_b64_url_nopad(signature_b64)?;

    let verifying_key: RsaVerifyingKey<Sha256> = RsaVerifyingKey::new(public_key);
    let signature = rsa::pkcs1v15::Signature::try_from(sig_bytes.as_slice())
        .map_err(|_| AuthError::InvalidSignature)?;

    verifying_key
        .verify(signing_input.as_bytes(), &signature)
        .map_err(|_| AuthError::InvalidSignature)
}

fn verify_oidc_claims(payload: &Value, expected_issuer: Option<&str>) -> Result<(), AuthError> {
    if let Some(expected_issuer) = expected_issuer {
        let iss = payload
            .get("iss")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::PublicKeyBuild("OIDC payload missing iss".into()))?;
        if iss != expected_issuer {
            return Err(AuthError::PublicKeyBuild(format!(
                "OIDC issuer mismatch: expected {expected_issuer}, got {iss}"
            )));
        }
    }

    let aud = payload
        .get("aud")
        .ok_or_else(|| AuthError::PublicKeyBuild("OIDC payload missing aud".into()))?;
    let aud_match = aud.as_str().map_or_else(
        || {
            aud.as_array()
                .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(OIDC_AUDIENCE)))
        },
        |s| s == OIDC_AUDIENCE,
    );
    if !aud_match {
        return Err(AuthError::PublicKeyBuild(format!(
            "OIDC audience mismatch: expected {OIDC_AUDIENCE}, got {aud:?}"
        )));
    }

    let exp = payload
        .get("exp")
        .and_then(Value::as_u64)
        .ok_or_else(|| AuthError::PublicKeyBuild("OIDC payload missing exp".into()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| AuthError::PublicKeyBuild("Clock before UNIX epoch".into()))?
        .as_secs();
    if now > exp {
        return Err(AuthError::PublicKeyBuild("OIDC token expired".into()));
    }

    Ok(())
}

/// Extracts `PlayerClaims` from an unverified OIDC token payload.
pub fn extract_oidc_token_player_claims(token: &str) -> Result<PlayerClaims, AuthError> {
    let mut parts = token.split('.');
    parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let payload_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let v: Value = serde_json::from_slice(&decode_b64_url_nopad(payload_b64)?)?;
    Ok(untrusted_player_claims(&v))
}

fn untrusted_player_claims(payload: &Value) -> PlayerClaims {
    let display_name = payload
        .get("xname")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // Offline/self-signed claims cannot link to a Java or authenticated Xbox identity.
    // Authenticated XUIDs retain Pumpkin's pocket-auth-1-xuid namespace.
    let input = format!("OfflineBedrockPlayer:{display_name}");
    let mut bytes = *md5::compute(input.as_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    PlayerClaims {
        display_name,
        uuid: uuid::Uuid::from_bytes(bytes).to_string(),
        xuid: String::new(),
    }
}

fn extract_oidc_player_claims(payload: &Value) -> PlayerClaims {
    let xuid = payload
        .get("xid")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if xuid.is_empty() {
        return untrusted_player_claims(payload);
    }
    PlayerClaims {
        display_name: payload
            .get("xname")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        uuid: xuid_to_uuid(xuid),
        xuid: xuid.to_owned(),
    }
}

/// Verifies Bedrock client data against the client key bound into the login token.
/// Both `RakNet` and `NetherNet` must call this before trusting skin or device claims.
pub fn verify_client_token(token: &str, public_key: &PublicKey) -> Result<Value, AuthError> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let payload = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let signature = parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    if parts.next().is_some() {
        return Err(AuthError::InvalidTokenFormat);
    }
    let header_json: Value = serde_json::from_slice(&decode_b64_url_nopad(header)?)?;
    if header_json.get("alg").and_then(Value::as_str) != Some("ES384") {
        return Err(AuthError::InvalidSignature);
    }
    verify_es384_signature(public_key, &format!("{header}.{payload}"), signature)?;
    Ok(serde_json::from_slice(&decode_b64_url_nopad(payload)?)?)
}

fn xuid_to_uuid(xuid: &str) -> String {
    let input = format!("pocket-auth-1-xuid:{xuid}");
    let mut bytes = *md5::compute(input.as_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

/// Extracts the client public key (cpk) from an OIDC token payload.
pub fn extract_cpk_from_token(token: &str) -> Result<PublicKey, AuthError> {
    let mut parts = token.split('.');
    parts.next().ok_or(AuthError::InvalidTokenFormat)?;
    let payload_b64 = parts.next().ok_or(AuthError::InvalidTokenFormat)?;

    let payload_bytes = decode_b64_url_nopad(payload_b64)?;
    let v: Value = serde_json::from_slice(&payload_bytes)?;

    let cpk_b64 = v
        .get("cpk")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::PublicKeyBuild("OIDC payload missing cpk".into()))?;

    build_public_key_from_b64(cpk_b64)
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn weak_bedrock_claims_cannot_select_java_or_verified_xbox_uuids() {
        let claim = serde_json::json!({"xname": "Steve", "xid": "123456", "leguuid": "069a79f4-44e9-4726-a5be-fca90e38aaf5"});
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claim).unwrap());
        let weak = extract_oidc_token_player_claims(&format!("e30.{payload}.unverified")).unwrap();
        let verified = extract_oidc_player_claims(&claim);
        assert_ne!(weak.uuid, "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        assert_ne!(weak.uuid, verified.uuid);
        assert!(weak.xuid.is_empty());
        let spoofed = serde_json::json!({"xname": "Steve", "xid": "987654", "leguuid": "00000000-0000-0000-0000-000000000000"});
        assert_eq!(weak.uuid, untrusted_player_claims(&spoofed).uuid);
    }

    #[test]
    fn bedrock_client_data_must_be_signed_by_the_bound_key() {
        // P-384 generator's uncompressed SEC1 representation, an independent public-key fixture.
        let key = build_public_key_from_b64("BKqHyiK+iwU3jrHHHvMgrXRuHTtii6ebmFn3QeCCVCo4VQLyXb9VKWw6VF44cnYKtzYX3kqWJixvXZ6Yv5KS3Cn49B29KJoUfOnaMRO18LjACmCxzh1+gZ16Qx18kOoOXw==").unwrap();
        let header = general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"ES384"}"#);
        let signature = general_purpose::URL_SAFE_NO_PAD.encode([0; 96]);
        assert!(verify_client_token(&format!("{header}.e30.{signature}"), &key).is_err());
        assert!(verify_client_token("e30.e30", &key).is_err());
    }

    #[test]
    fn bedrock_client_token_accepts_the_bound_signature_and_rejects_tampering() {
        use p384::ecdsa::{SigningKey, signature::Signer};
        let private = SigningKey::from_slice(&[1; 48]).unwrap();
        let public =
            PublicKey::from_sec1_bytes(private.verifying_key().to_sec1_point(false).as_bytes())
                .unwrap();
        let header = general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"ES384"}"#);
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(br#"{"LanguageCode":"en_US"}"#);
        let input = format!("{header}.{payload}");
        let signature: p384::ecdsa::Signature = private.sign(input.as_bytes());
        let token = format!(
            "{input}.{}",
            general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes())
        );
        assert_eq!(
            verify_client_token(&token, &public).unwrap()["LanguageCode"],
            "en_US"
        );
        let altered = token.replace(&payload, "e30");
        assert!(verify_client_token(&altered, &public).is_err());
        let other = SigningKey::from_slice(&[2; 48]).unwrap();
        let other_public =
            PublicKey::from_sec1_bytes(other.verifying_key().to_sec1_point(false).as_bytes())
                .unwrap();
        assert!(verify_client_token(&token, &other_public).is_err());
    }
}
