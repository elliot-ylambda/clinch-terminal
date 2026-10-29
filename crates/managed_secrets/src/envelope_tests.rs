use std::io;

use tink_proto::KeysetInfo;
use tink_proto::keyset_info::KeyInfo;

use super::UploadKey;
use crate::ManagedSecretValue;

#[test]
fn p256_ecies_roundtrip_rejects_modified_context_and_ciphertext() {
    super::init();
    for template in [
        tink_hybrid::ecies_hkdf_aes128_gcm_key_template(),
        tink_hybrid::ecies_hkdf_aes128_ctr_hmac_sha256_key_template(),
    ] {
        let private = tink_core::keyset::Handle::new(&template).unwrap();
        let public = private.public().unwrap();
        let encrypt = tink_hybrid::new_encrypt(&public).unwrap();
        let decrypt = tink_hybrid::new_decrypt(&private).unwrap();
        let plaintext = b"P-256 compatibility regression";
        let context = b"managed-secret context";
        let mut ciphertext = encrypt.encrypt(plaintext, context).unwrap();
        assert_eq!(decrypt.decrypt(&ciphertext, context).unwrap(), plaintext);
        assert!(decrypt.decrypt(&ciphertext, b"wrong context").is_err());
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;
        assert!(decrypt.decrypt(&ciphertext, context).is_err());
    }
}

#[test]
fn p256_point_and_scalar_compatibility() {
    use tink_hybrid::subtle::{EcPrivateKey, EcPublicKey, compute_shared_secret};
    use tink_proto::EllipticCurveType::NistP256;

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }

    // Valid point from upstream Tink's elliptic_curves_test.rs (Apache-2.0).
    let x = bytes("700c48f77f56584c5cc632ca65640db91b6bacce3a4df6b42ce7cc838833d287");
    let mut y = bytes("db71e509e3fd9b060ddb20ba5c51dcc5948d46fbf640dfe0441782cab85fa4ac");
    let public = EcPublicKey::new(NistP256, &x, &y).unwrap();
    assert_eq!(public.x_y_bytes().unwrap(), (x.clone(), y.clone()));
    // Multiplication by scalar one preserves the peer point's x coordinate.
    let private = EcPrivateKey::new(NistP256, &[1]).unwrap();
    assert_eq!(compute_shared_secret(&public, &private).unwrap(), x);
    let mut padded = vec![0; 33];
    padded[32] = 1;
    let padded_private = EcPrivateKey::new(NistP256, &padded).unwrap();
    assert_eq!(compute_shared_secret(&public, &padded_private).unwrap(), x);
    padded[0] = 1;
    assert!(EcPrivateKey::new(NistP256, &padded).is_err());
    assert!(EcPrivateKey::new(NistP256, &[0]).is_err());
    y[31] ^= 1;
    assert!(EcPublicKey::new(NistP256, &x, &y).is_err());
}

#[test]
fn test_import_public_keyset() {
    super::init();

    // A base64-encoded public keyset as returned by warp-server.
    let public_key = "COvInMIBEnAKZAo0dHlwZS5nb29nbGVhcGlzLmNvbS9nb29nbGUuY3J5cHRvLnRpbmsuSHBrZVB1YmxpY0tleRIqEgYIARABGAIaIHRaibhtYbpEfh2CSpdDPhh/6lCBnfoO3nqBmZ3VQGJyGAMQARjryJzCASAB";
    let upload_key =
        UploadKey::import_public_keyset(public_key).expect("unable to import public keyset");

    let keyset_info = upload_key.public_key.keyset_info();
    assert_eq!(
        keyset_info,
        KeysetInfo {
            primary_key_id: 407315563,
            key_info: vec![KeyInfo {
                key_id: 407315563,
                status: tink_proto::KeyStatusType::Enabled.into(),
                type_url: "type.googleapis.com/google.crypto.tink.HpkePublicKey".to_string(),
                output_prefix_type: tink_proto::OutputPrefixType::Tink.into(),
            }],
        }
    );

    let encrypted = upload_key
        .encrypt
        .encrypt(b"hello from rust", b"rust context")
        .expect("unable to encrypt");
    assert!(!encrypted.is_empty());
}

/// An HPKE private key for use in tests.
///
/// Created with:
/// ```sh
/// $ java -jar /opt/homebrew/Cellar/tinkey/1.12.0/bin/tinkey_deploy.jar create-keyset --key-template DHKEM_X25519_HKDF_SHA256_HKDF_SHA256_AES_256_GCM --out-format json | jq .
/// ```
const TEST_PRIVATE_KEY: &str = r#"
{
  "primaryKeyId": 625520774,
  "key": [
    {
      "keyData": {
        "typeUrl": "type.googleapis.com/google.crypto.tink.HpkePrivateKey",
        "value": "EioSBggBEAEYAhogGHVh0Tju/DHOWEgpuUJ+9P/pXa5tK16udRWoJJwHbnIaIHdq5FthS7H4Q6xSLzCEnbf1z/F1+PTQHev/5PJ+pc+m",
        "keyMaterialType": "ASYMMETRIC_PRIVATE"
      },
      "status": "ENABLED",
      "keyId": 625520774,
      "outputPrefixType": "TINK"
    }
  ]
}
"#;

/// An HPKE public key for use in tests, corresponding to [`TEST_PRIVATE_KEY`].
///
/// Created with:
/// ```sh
/// $ java -jar /opt/homebrew/Cellar/tinkey/1.12.0/bin/tinkey_deploy.jar create-public-keyset | jq .
/// < private key JSON on stdin >
/// ```
const TEST_PUBLIC_KEY: &str = r#"
{
  "primaryKeyId": 625520774,
  "key": [
    {
      "keyData": {
        "typeUrl": "type.googleapis.com/google.crypto.tink.HpkePublicKey",
        "value": "EgYIARABGAIaIBh1YdE47vwxzlhIKblCfvT/6V2ubSternUVqCScB25y",
        "keyMaterialType": "ASYMMETRIC_PUBLIC"
      },
      "status": "ENABLED",
      "keyId": 625520774,
      "outputPrefixType": "TINK"
    }
  ]
}
"#;

/// Test encrypting a managed secret value.
#[test]
fn test_encrypt_managed_secret() {
    super::init();

    let keyset = read_keyset_json(TEST_PUBLIC_KEY);
    let upload_key = UploadKey {
        encrypt: tink_hybrid::new_encrypt(&keyset).expect("failed to create encrypt primitive"),
        public_key: keyset,
    };

    let encrypted = upload_key
        .encrypt_secret(
            "user123",
            "MY_SECRET",
            &ManagedSecretValue::RawValue {
                value: "secret".to_string(),
            },
        )
        .expect("failed to encrypt secret");
    assert!(!encrypted.is_empty());
}

/// Test our HPKE encryption and decryption primitives. At the very least, they should be able to roundtrip a plaintext value.
#[test]
fn test_encrypt_decrypt() {
    super::init();

    let private_key = read_keyset_json(TEST_PRIVATE_KEY);
    let public_key = read_keyset_json(TEST_PUBLIC_KEY);

    let encrypt =
        tink_hybrid::new_encrypt(&public_key).expect("failed to create encrypt primitive");
    let decrypt =
        tink_hybrid::new_decrypt(&private_key).expect("failed to create decrypt primitive");

    let context = b"I am context";
    let plaintext = b"hello from rust";

    let encrypted = encrypt
        .encrypt(plaintext, context)
        .expect("failed to encrypt");
    let decrypted = decrypt
        .decrypt(&encrypted, context)
        .expect("failed to decrypt");

    assert_eq!(decrypted, plaintext);
}

fn read_keyset_json(json: &str) -> tink_core::keyset::Handle {
    let mut reader = tink_core::keyset::JsonReader::new(io::Cursor::new(json.as_bytes()));
    tink_core::keyset::insecure::read(&mut reader).expect("failed to read keyset")
}
