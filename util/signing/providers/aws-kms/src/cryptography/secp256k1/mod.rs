use aws_sdk_kms::types::{KeySpec, KeyUsageType, SigningAlgorithmSpec};
use movement_signer::cryptography::secp256k1::Secp256k1;

/// Defines the needed methods for providing a definition of cryptography used with AWS KMS
pub trait AwsKmsCryptography {
	/// Returns the [KeySpec] for the desired cryptography
	fn key_spec() -> KeySpec;

	/// Returns the [KeyUsageType] for the desired cryptography
	fn key_usage_type() -> KeyUsageType;

	/// Returns the [SigningAlgorithmSpec] for the desired cryptography
	fn signing_algorithm_spec() -> SigningAlgorithmSpec;
}

impl AwsKmsCryptography for Secp256k1 {
	fn key_spec() -> KeySpec {
		KeySpec::EccSecgP256K1
	}

	fn key_usage_type() -> KeyUsageType {
		KeyUsageType::SignVerify
	}

	fn signing_algorithm_spec() -> SigningAlgorithmSpec {
		SigningAlgorithmSpec::EcdsaSha256
	}
}
