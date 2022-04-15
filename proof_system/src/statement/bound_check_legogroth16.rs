use ark_ec::{AffineCurve, PairingEngine};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, SerializationError};
use ark_std::io::{Read, Write};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

pub use legogroth16::{PreparedVerifyingKey, ProvingKey, VerifyingKey};

use crate::error::ProofSystemError;
use crate::setup_params::SetupParams;
use crate::statement::Statement;
use crate::sub_protocols::bound_check::BoundCheckProtocol;
use dock_crypto_utils::serde_utils::*;

pub use serialization::*;

/// Proving knowledge of message that satisfies given bounds, i.e. min <= message <= max
#[serde_as]
#[derive(
    Clone, Debug, PartialEq, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
#[serde(bound = "")]
pub struct BoundCheckLegoGroth16Prover<E: PairingEngine> {
    #[serde_as(as = "FieldBytes")]
    pub min: E::Fr,
    #[serde_as(as = "FieldBytes")]
    pub max: E::Fr,
    #[serde_as(as = "Option<LegoProvingKeyBytes>")]
    pub snark_proving_key: Option<ProvingKey<E>>,
    pub snark_proving_key_ref: Option<usize>,
}

/// Proving knowledge of message that satisfies given bounds, i.e. min <= message <= max
#[serde_as]
#[derive(
    Clone, Debug, PartialEq, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
#[serde(bound = "")]
pub struct BoundCheckLegoGroth16Verifier<E: PairingEngine> {
    #[serde_as(as = "FieldBytes")]
    pub min: E::Fr,
    #[serde_as(as = "FieldBytes")]
    pub max: E::Fr,
    #[serde_as(as = "Option<LegoVerifyingKeyBytes>")]
    pub snark_verifying_key: Option<VerifyingKey<E>>,
    pub snark_verifying_key_ref: Option<usize>,
}

impl<E: PairingEngine> BoundCheckLegoGroth16Prover<E> {
    pub fn new_statement_from_params<G: AffineCurve>(
        min: E::Fr,
        max: E::Fr,
        snark_proving_key: ProvingKey<E>,
    ) -> Result<Statement<E, G>, ProofSystemError> {
        BoundCheckProtocol::validate_bounds(min, max, &snark_proving_key.vk)?;

        Ok(Statement::BoundCheckLegoGroth16Prover(Self {
            min,
            max,
            snark_proving_key: Some(snark_proving_key),
            snark_proving_key_ref: None,
        }))
    }

    pub fn new_statement_from_params_ref<G: AffineCurve>(
        min: E::Fr,
        max: E::Fr,
        snark_proving_key_ref: usize,
    ) -> Statement<E, G> {
        Statement::BoundCheckLegoGroth16Prover(Self {
            min,
            max,
            snark_proving_key: None,
            snark_proving_key_ref: Some(snark_proving_key_ref),
        })
    }

    pub fn get_proving_key<'a, G: AffineCurve>(
        &'a self,
        setup_params: &'a [SetupParams<E, G>],
        st_idx: usize,
    ) -> Result<&'a ProvingKey<E>, ProofSystemError> {
        extract_param!(
            setup_params,
            &self.snark_proving_key,
            self.snark_proving_key_ref,
            LegoSnarkProvingKey,
            IncompatibleBoundCheckSetupParamAtIndex,
            st_idx
        )
    }
}

impl<E: PairingEngine> BoundCheckLegoGroth16Verifier<E> {
    pub fn new_statement_from_params<G: AffineCurve>(
        min: E::Fr,
        max: E::Fr,
        snark_verifying_key: VerifyingKey<E>,
    ) -> Result<Statement<E, G>, ProofSystemError> {
        BoundCheckProtocol::validate_bounds(min, max, &snark_verifying_key)?;

        Ok(Statement::BoundCheckLegoGroth16Verifier(Self {
            min,
            max,
            snark_verifying_key: Some(snark_verifying_key),
            snark_verifying_key_ref: None,
        }))
    }

    pub fn new_statement_from_params_ref<G: AffineCurve>(
        min: E::Fr,
        max: E::Fr,
        snark_verifying_key_ref: usize,
    ) -> Statement<E, G> {
        Statement::BoundCheckLegoGroth16Verifier(Self {
            min,
            max,
            snark_verifying_key: None,
            snark_verifying_key_ref: Some(snark_verifying_key_ref),
        })
    }

    pub fn get_verifying_key<'a, G: AffineCurve>(
        &'a self,
        setup_params: &'a [SetupParams<E, G>],
        st_idx: usize,
    ) -> Result<&'a VerifyingKey<E>, ProofSystemError> {
        extract_param!(
            setup_params,
            &self.snark_verifying_key,
            self.snark_verifying_key_ref,
            LegoSnarkVerifyingKey,
            IncompatibleBoundCheckSetupParamAtIndex,
            st_idx
        )
    }
}

mod serialization {
    use super::*;
    use ark_std::{fmt, marker::PhantomData, vec, vec::Vec};
    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};
    use serde_with::{DeserializeAs, SerializeAs};

    impl_for_groth16_struct!(LegoProvingKeyBytes, ProvingKey, "expected LegoProvingKey");

    impl_for_groth16_struct!(
        LegoVerifyingKeyBytes,
        VerifyingKey,
        "expected LegoVerifyingKey"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sub_protocols::bound_check::generate_snark_srs_bound_check;
    use ark_bls12_381::fr::Fr;
    use ark_bls12_381::Bls12_381;
    use ark_ff::{BigInteger, One, PrimeField};
    use ark_std::{
        rand::{rngs::StdRng, SeedableRng},
        UniformRand,
    };

    #[test]
    fn bound_check_statement_validity() {
        let mut rng = StdRng::seed_from_u64(0u64);
        let snark_pk = generate_snark_srs_bound_check::<Bls12_381, _>(&mut rng).unwrap();
        assert!(BoundCheckLegoGroth16Prover::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(5u64), snark_pk.clone())
        .is_err());
        assert!(BoundCheckLegoGroth16Verifier::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(5u64), snark_pk.vk.clone())
        .is_err());
        assert!(BoundCheckLegoGroth16Prover::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(4u64), snark_pk.clone())
        .is_err());
        assert!(BoundCheckLegoGroth16Verifier::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(4u64), snark_pk.vk.clone())
        .is_err());
        assert!(BoundCheckLegoGroth16Prover::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(6u64), snark_pk.clone())
        .is_ok());
        assert!(BoundCheckLegoGroth16Verifier::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(6u64), snark_pk.vk.clone())
        .is_ok());
        let max_allowed = Fr::modulus_minus_one_div_two();
        let mut max = max_allowed.clone();
        max.add_nocarry(&Fr::one().into_repr());
        assert!(BoundCheckLegoGroth16Prover::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(max), snark_pk.clone())
        .is_err());
        assert!(BoundCheckLegoGroth16Verifier::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(max), snark_pk.vk.clone())
        .is_err());
        assert!(BoundCheckLegoGroth16Prover::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(max_allowed), snark_pk.clone())
        .is_ok());
        assert!(BoundCheckLegoGroth16Verifier::new_statement_from_params::<
            <Bls12_381 as PairingEngine>::G1Affine,
        >(Fr::from(5u64), Fr::from(max_allowed), snark_pk.vk.clone())
        .is_ok());
    }
}
