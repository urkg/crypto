#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! Zero knowledge proof protocols for membership and non-membership witnesses from section 7 of the paper
//! # Examples
//!
//! ```
//! use ark_bls12_381::Bls12_381;
//! use vb_accumulator::setup::{Keypair, SetupParams};
//! use vb_accumulator::positive::{PositiveAccumulator, Accumulator};
//! use vb_accumulator::witness::MembershipWitness;
//! use vb_accumulator::proofs::{MembershipProofProtocol, MembershipProvingKey};
//! use vb_accumulator::persistence::State;
//!
//! let params = SetupParams::<Bls12_381>::generate_using_rng(&mut rng);
//! let keypair = Keypair::<Bls12_381>::generate(&mut rng, &params);
//!
//! let accumulator = PositiveAccumulator::initialize(&params);
//!
//! // Add elements
//!
//! // Create membership witness for existing `elem`
//! let m_wit = accumulator
//!                 .get_membership_witness(&elem, &keypair.secret_key, &state)
//!                 .unwrap();
//!
//! // The prover and verifier should agree on the proving key
//! let prk = MembershipProvingKey::generate_using_rng(&mut rng);
//!
//! // Prover initializes the protocol
//! let protocol = MembershipProofProtocol::init(
//!                 &mut rng,
//!                 &elem,
//!                 None,
//!                 &m_wit,
//!                 &keypair.public_key,
//!                 &params,
//!                 &prk,
//!             );
//!
//! // `challenge_bytes` is the stream where the protocol's challenge contribution will be written
//!
//! protocol
//!                 .challenge_contribution(
//!                     accumulator.value(),
//!                     &keypair.public_key,
//!                     &params,
//!                     &prk,
//!                     &mut challenge_bytes,
//!                 )
//!                 .unwrap();
//!
//! // Generate `challenge` from `challenge_bytes`, see tests for example
//!
//! let proof = protocol.gen_proof(&challenge);
//!
//! // Verifier should independently generate the `challenge`
//!
//! // `challenge_bytes` is the stream where the proof's challenge contribution will be written
//! proof
//!                 .challenge_contribution(
//!                     accumulator.value(),
//!                     &keypair.public_key,
//!                     &params,
//!                     &prk,
//!                     &mut chal_bytes_verifier,
//!                 )
//!                 .unwrap();
//!
//! // Generate `challenge` from `challenge_bytes`, see tests for example
//! proof
//!                 .verify(
//!                     &accumulator.value(),
//!                     &challenge,
//!                     &keypair.public_key,
//!                     &params,
//!                     &prk,
//!                 )
//!                 .unwrap();
//!
//! // Non-membership proof has a similar API, see tests for example.
//! ```

use crate::error::VBAccumulatorError;
use crate::setup::{PublicKey, SetupParams};
use crate::witness::{MembershipWitness, NonMembershipWitness};
use ark_ec::wnaf::WnafContext;
use ark_ec::{AffineCurve, PairingEngine, ProjectiveCurve};
use ark_ff::{to_bytes, Field, PrimeField, SquareRootField, Zero};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, SerializationError};
use ark_std::{
    fmt::Debug,
    io::{Read, Write},
    rand::RngCore,
    UniformRand,
};
use digest::Digest;
use dock_crypto_utils::hashing_utils::projective_group_elem_from_try_and_incr;
use dock_crypto_utils::serde_utils::*;
use schnorr_pok::error::SchnorrError;
use schnorr_pok::SchnorrChallengeContributor;

use crate::utils::WindowTable;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// The public parameters (in addition to public key, accumulator setup params) used during the proof
/// of membership and non-membership are called `ProvingKey`. These are mutually agreed upon by the
/// prover and verifier and can be different between different provers and verifiers but using the
/// same accumulator parameters and keys.
/// Common elements of the membership and non-membership proving key
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct ProvingKey<G: AffineCurve> {
    #[serde_as(as = "AffineGroupBytes")]
    pub X: G,
    #[serde_as(as = "AffineGroupBytes")]
    pub Y: G,
    #[serde_as(as = "AffineGroupBytes")]
    pub Z: G,
}

/// Used between prover and verifier only to prove knowledge of member and corresponding witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct MembershipProvingKey<G: AffineCurve>(
    #[serde(bound = "ProvingKey<G>: Serialize, for<'a> ProvingKey<G>: Deserialize<'a>")]
    pub  ProvingKey<G>,
);

/// Used between prover and verifier only to prove knowledge of non-member and corresponding witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct NonMembershipProvingKey<G: AffineCurve> {
    #[serde(bound = "ProvingKey<G>: Serialize, for<'a> ProvingKey<G>: Deserialize<'a>")]
    pub XYZ: ProvingKey<G>,
    #[serde_as(as = "AffineGroupBytes")]
    pub K: G,
}

impl<G> ProvingKey<G>
where
    G: AffineCurve,
{
    /// Generate using a random number generator
    fn generate_proving_key_using_rng<R: RngCore>(rng: &mut R) -> ProvingKey<G> {
        ProvingKey {
            X: G::Projective::rand(rng).into(),
            Y: G::Projective::rand(rng).into(),
            Z: G::Projective::rand(rng).into(),
        }
    }

    /// Generate by hashing known strings
    fn generate_proving_key_using_hash<D: Digest>(label: &[u8]) -> ProvingKey<G> {
        // 3 G1 elements
        let mut elems: [G::Projective; 3] = [
            projective_group_elem_from_try_and_incr::<G, D>(
                &to_bytes![label, " : X".as_bytes()].unwrap(),
            ),
            projective_group_elem_from_try_and_incr::<G, D>(
                &to_bytes![label, " : Y".as_bytes()].unwrap(),
            ),
            projective_group_elem_from_try_and_incr::<G, D>(
                &to_bytes![label, " : Z".as_bytes()].unwrap(),
            ),
        ];
        G::Projective::batch_normalization(&mut elems);
        let [X, Y, Z] = [elems[0].into(), elems[1].into(), elems[2].into()];

        ProvingKey { X, Y, Z }
    }
}

impl<G> MembershipProvingKey<G>
where
    G: AffineCurve,
{
    /// Generate using a random number generator
    pub fn generate_using_rng<R: RngCore>(rng: &mut R) -> Self {
        Self(ProvingKey::generate_proving_key_using_rng(rng))
    }

    /// Generate by hashing known strings
    pub fn new<D: Digest>(label: &[u8]) -> Self {
        Self(ProvingKey::generate_proving_key_using_hash::<D>(label))
    }
}

impl<G> NonMembershipProvingKey<G>
where
    G: AffineCurve,
{
    /// Generate using a random number generator
    pub fn generate_using_rng<R: RngCore>(rng: &mut R) -> Self {
        let XYZ = ProvingKey::generate_proving_key_using_rng(rng);
        Self {
            XYZ,
            K: G::Projective::rand(rng).into(),
        }
    }

    /// Generate by hashing known strings
    pub fn new<D: Digest>(label: &[u8]) -> Self {
        let XYZ = ProvingKey::generate_proving_key_using_hash::<D>(label);
        Self {
            XYZ,
            K: projective_group_elem_from_try_and_incr::<G, D>(
                &to_bytes![label, " : K".as_bytes()].unwrap(),
            )
            .into(),
        }
    }

    /// Derive the membership proving key when doing a membership proof with a universal accumulator.
    pub fn derive_membership_proving_key(&self) -> MembershipProvingKey<G> {
        MembershipProvingKey(self.XYZ.clone())
    }
}

/// Common elements of the randomized witness between membership and non-membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct RandomizedWitness<G: AffineCurve> {
    #[serde_as(as = "AffineGroupBytes")]
    pub E_C: G,
    #[serde_as(as = "AffineGroupBytes")]
    pub T_sigma: G,
    #[serde_as(as = "AffineGroupBytes")]
    pub T_rho: G,
}

/// Common elements of the blindings (Schnorr protocol) between membership and non-membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct Blindings<F: PrimeField + SquareRootField> {
    #[serde_as(as = "FieldBytes")]
    pub sigma: F,
    #[serde_as(as = "FieldBytes")]
    pub rho: F,
    #[serde_as(as = "FieldBytes")]
    pub delta_sigma: F,
    #[serde_as(as = "FieldBytes")]
    pub delta_rho: F,
    #[serde_as(as = "FieldBytes")]
    pub r_y: F,
    #[serde_as(as = "FieldBytes")]
    pub r_sigma: F,
    #[serde_as(as = "FieldBytes")]
    pub r_rho: F,
    #[serde_as(as = "FieldBytes")]
    pub r_delta_sigma: F,
    #[serde_as(as = "FieldBytes")]
    pub r_delta_rho: F,
}

/// Common elements of the commitment (Schnorr protocol, step 1) between membership and non-membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct SchnorrCommit<E: PairingEngine> {
    #[serde_as(as = "FieldBytes")]
    pub R_E: E::Fqk,
    #[serde_as(as = "AffineGroupBytes")]
    pub R_sigma: E::G1Affine,
    #[serde_as(as = "AffineGroupBytes")]
    pub R_rho: E::G1Affine,
    #[serde_as(as = "AffineGroupBytes")]
    pub R_delta_sigma: E::G1Affine,
    #[serde_as(as = "AffineGroupBytes")]
    pub R_delta_rho: E::G1Affine,
}

/// Common elements of the response (Schnorr protocol, step 3) between membership and non-membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct SchnorrResponse<F: PrimeField + SquareRootField> {
    #[serde_as(as = "FieldBytes")]
    pub s_y: F,
    #[serde_as(as = "FieldBytes")]
    pub s_sigma: F,
    #[serde_as(as = "FieldBytes")]
    pub s_rho: F,
    #[serde_as(as = "FieldBytes")]
    pub s_delta_sigma: F,
    #[serde_as(as = "FieldBytes")]
    pub s_delta_rho: F,
}

/// Randomized membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct MembershipRandomizedWitness<G: AffineCurve>(
    #[serde(
        bound = "RandomizedWitness<G>: Serialize, for<'a> RandomizedWitness<G>: Deserialize<'a>"
    )]
    pub RandomizedWitness<G>,
);

/// Blindings used during membership proof protocol
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct MembershipBlindings<F: PrimeField + SquareRootField>(
    #[serde(bound = "Blindings<F>: Serialize, for<'a> Blindings<F>: Deserialize<'a>")]
    pub  Blindings<F>,
);

/// Commitments from various Schnorr protocols used during membership proof protocol
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct MembershipSchnorrCommit<E: PairingEngine>(
    #[serde(bound = "SchnorrCommit<E>: Serialize, for<'a> SchnorrCommit<E>: Deserialize<'a>")]
    pub  SchnorrCommit<E>,
);

/// Responses from various Schnorr protocols used during membership proof protocol
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct MembershipSchnorrResponse<F: PrimeField + SquareRootField>(
    #[serde(bound = "SchnorrResponse<F>: Serialize, for<'a> SchnorrResponse<F>: Deserialize<'a>")]
    pub SchnorrResponse<F>,
);

/// Proof of knowledge of the member and the membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct MembershipProof<E: PairingEngine> {
    #[serde(
        bound = "MembershipRandomizedWitness<E::G1Affine>: Serialize, for<'a> MembershipRandomizedWitness<E::G1Affine>: Deserialize<'a>"
    )]
    pub randomized_witness: MembershipRandomizedWitness<E::G1Affine>,
    #[serde(
        bound = "MembershipSchnorrCommit<E>: Serialize, for<'a> MembershipSchnorrCommit<E>: Deserialize<'a>"
    )]
    pub schnorr_commit: MembershipSchnorrCommit<E>,
    #[serde(
        bound = "MembershipSchnorrResponse<E::Fr>: Serialize, for<'a> MembershipSchnorrResponse<E::Fr>: Deserialize<'a>"
    )]
    pub schnorr_response: MembershipSchnorrResponse<E::Fr>,
}

/// Protocol for proving knowledge of the member and the membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct MembershipProofProtocol<E: PairingEngine> {
    #[serde_as(as = "FieldBytes")]
    pub element: E::Fr,
    #[serde(
        bound = "MembershipRandomizedWitness<E::G1Affine>: Serialize, for<'a> MembershipRandomizedWitness<E::G1Affine>: Deserialize<'a>"
    )]
    pub randomized_witness: MembershipRandomizedWitness<E::G1Affine>,
    #[serde(
        bound = "MembershipSchnorrCommit<E>: Serialize, for<'a> MembershipSchnorrCommit<E>: Deserialize<'a>"
    )]
    pub schnorr_commit: MembershipSchnorrCommit<E>,
    #[serde(
        bound = "MembershipSchnorrResponse<E::Fr>: Serialize, for<'a> MembershipSchnorrResponse<E::Fr>: Deserialize<'a>"
    )]
    pub schnorr_blindings: MembershipBlindings<E::Fr>,
}

/// Randomized non-membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct NonMembershipRandomizedWitness<G: AffineCurve> {
    #[serde(
        bound = "RandomizedWitness<G>: Serialize, for<'a> RandomizedWitness<G>: Deserialize<'a>"
    )]
    pub C: RandomizedWitness<G>,
    #[serde_as(as = "AffineGroupBytes")]
    pub E_d: G,
    #[serde_as(as = "AffineGroupBytes")]
    pub E_d_inv: G,
}

/// Blindings used during non-membership proof protocol
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct NonMembershipBlindings<F: PrimeField + SquareRootField> {
    #[serde(bound = "Blindings<F>: Serialize, for<'a> Blindings<F>: Deserialize<'a>")]
    pub C: Blindings<F>,
    #[serde_as(as = "FieldBytes")]
    pub tau: F,
    #[serde_as(as = "FieldBytes")]
    pub pi: F,
    #[serde_as(as = "FieldBytes")]
    pub r_u: F,
    #[serde_as(as = "FieldBytes")]
    pub r_v: F,
    #[serde_as(as = "FieldBytes")]
    pub r_w: F,
}

/// Commitments from various Schnorr protocols used during non-membership proof protocol
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct NonMembershipSchnorrCommit<E: PairingEngine> {
    #[serde(bound = "SchnorrCommit<E>: Serialize, for<'a> SchnorrCommit<E>: Deserialize<'a>")]
    pub C: SchnorrCommit<E>,
    #[serde_as(as = "AffineGroupBytes")]
    pub R_A: E::G1Affine,
    #[serde_as(as = "AffineGroupBytes")]
    pub R_B: E::G1Affine,
}

/// Responses from various Schnorr protocols used during non-membership proof protocol
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct NonMembershipSchnorrResponse<F: PrimeField + SquareRootField> {
    #[serde(bound = "SchnorrResponse<F>: Serialize, for<'a> SchnorrResponse<F>: Deserialize<'a>")]
    pub C: SchnorrResponse<F>,
    #[serde_as(as = "FieldBytes")]
    pub s_u: F,
    #[serde_as(as = "FieldBytes")]
    pub s_v: F,
    #[serde_as(as = "FieldBytes")]
    pub s_w: F,
}

/// Proof of knowledge of the non-member and the non-membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct NonMembershipProof<E: PairingEngine> {
    #[serde(
        bound = "NonMembershipRandomizedWitness<E::G1Affine>: Serialize, for<'a> NonMembershipRandomizedWitness<E::G1Affine>: Deserialize<'a>"
    )]
    pub randomized_witness: NonMembershipRandomizedWitness<E::G1Affine>,
    #[serde(
        bound = "NonMembershipSchnorrCommit<E>: Serialize, for<'a> NonMembershipSchnorrCommit<E>: Deserialize<'a>"
    )]
    pub schnorr_commit: NonMembershipSchnorrCommit<E>,
    #[serde(
        bound = "NonMembershipBlindings<E::Fr>: Serialize, for<'a> NonMembershipBlindings<E::Fr>: Deserialize<'a>"
    )]
    pub schnorr_response: NonMembershipSchnorrResponse<E::Fr>,
}

/// Protocol for proving knowledge of the non-member and the non-membership witness
#[serde_as]
#[derive(
    Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize, Serialize, Deserialize,
)]
pub struct NonMembershipProofProtocol<E: PairingEngine> {
    #[serde_as(as = "FieldBytes")]
    pub element: E::Fr,
    #[serde_as(as = "FieldBytes")]
    pub d: E::Fr,
    #[serde(
        bound = "NonMembershipRandomizedWitness<E::G1Affine>: Serialize, for<'a> NonMembershipRandomizedWitness<E::G1Affine>: Deserialize<'a>"
    )]
    pub randomized_witness: NonMembershipRandomizedWitness<E::G1Affine>,
    #[serde(
        bound = "NonMembershipSchnorrCommit<E>: Serialize, for<'a> NonMembershipSchnorrCommit<E>: Deserialize<'a>"
    )]
    pub schnorr_commit: NonMembershipSchnorrCommit<E>,
    #[serde(
        bound = "NonMembershipBlindings<E::Fr>: Serialize, for<'a> NonMembershipBlindings<E::Fr>: Deserialize<'a>"
    )]
    pub schnorr_blindings: NonMembershipBlindings<E::Fr>,
}

impl<G> SchnorrChallengeContributor for RandomizedWitness<G>
where
    G: AffineCurve,
{
    fn challenge_contribution<W: Write>(&self, mut writer: W) -> Result<(), SchnorrError> {
        self.E_C.serialize_unchecked(&mut writer)?;
        self.T_sigma.serialize_unchecked(&mut writer)?;
        self.T_rho
            .serialize_unchecked(&mut writer)
            .map_err(|e| e.into())
    }
}

impl<E> SchnorrChallengeContributor for SchnorrCommit<E>
where
    E: PairingEngine,
{
    fn challenge_contribution<W: Write>(&self, mut writer: W) -> Result<(), SchnorrError> {
        self.R_E.serialize_unchecked(&mut writer)?;
        self.R_sigma.serialize_unchecked(&mut writer)?;
        self.R_rho.serialize_unchecked(&mut writer)?;
        self.R_delta_sigma.serialize_unchecked(&mut writer)?;
        self.R_delta_rho
            .serialize_unchecked(&mut writer)
            .map_err(|e| e.into())
    }
}

impl<G> SchnorrChallengeContributor for MembershipRandomizedWitness<G>
where
    G: AffineCurve,
{
    fn challenge_contribution<W: Write>(&self, writer: W) -> Result<(), SchnorrError> {
        self.0.challenge_contribution(writer)
    }
}

impl<E> SchnorrChallengeContributor for MembershipSchnorrCommit<E>
where
    E: PairingEngine,
{
    fn challenge_contribution<W: Write>(&self, writer: W) -> Result<(), SchnorrError> {
        self.0.challenge_contribution(writer)
    }
}

impl<G> SchnorrChallengeContributor for NonMembershipRandomizedWitness<G>
where
    G: AffineCurve,
{
    fn challenge_contribution<W: Write>(&self, mut writer: W) -> Result<(), SchnorrError> {
        self.C.challenge_contribution(&mut writer)?;
        self.E_d.serialize_unchecked(&mut writer)?;
        self.E_d_inv
            .serialize_unchecked(&mut writer)
            .map_err(|e| e.into())
    }
}

impl<E> SchnorrChallengeContributor for NonMembershipSchnorrCommit<E>
where
    E: PairingEngine,
{
    fn challenge_contribution<W: Write>(&self, mut writer: W) -> Result<(), SchnorrError> {
        self.C.challenge_contribution(&mut writer)?;
        self.R_A.serialize_unchecked(&mut writer)?;
        self.R_B
            .serialize_unchecked(&mut writer)
            .map_err(|e| e.into())
    }
}

impl<F: PrimeField + SquareRootField> SchnorrResponse<F> {
    pub fn get_response_for_element(&self) -> &F {
        &self.s_y
    }
}

// TODO: Window size should not be hardcoded. It can be inferred from `ProvingKey` elements of proving key.

/// Protocol to prove knowledge of (non)member and corresponding witness in zero knowledge. It randomizes
/// the witness and does Schnorr proofs of knowledge of these randomized witness and the (non)member.
trait ProofProtocol<E: PairingEngine> {
    /// Randomize the witness and compute commitments for step 1 of the Schnorr protocol.
    /// `element` is the accumulator (non)member about which the proof is being created.
    /// `element_blinding` is the randomness used for `element` in the Schnorr protocol and is useful
    /// when `element` is used in some other relation as well.
    /// `pairing_extra` is used when creating non-membership proofs and is included in this function
    /// only because its efficient to do a multi-pairing.
    fn randomize_witness_and_compute_commitments<R: RngCore>(
        rng: &mut R,
        element: &E::Fr,
        element_blinding: Option<E::Fr>,
        witness: &E::G1Affine,
        pairing_extra: Option<E::G1Affine>,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &ProvingKey<E::G1Affine>,
    ) -> (
        RandomizedWitness<E::G1Affine>,
        SchnorrCommit<E>,
        Blindings<E::Fr>,
    ) {
        // TODO: Since proving key is fixed, these tables can be created just once and stored.
        // There are multiple multiplications with X, Y and Z so create tables for them. 20 multiplications
        // is the upper bound
        let scalar_size = E::Fr::size_in_bits();
        let X_table = WindowTable::new(scalar_size, 20, prk.X.into_projective());
        let Y_table = WindowTable::new(scalar_size, 20, prk.Y.into_projective());
        let Z_table = WindowTable::new(scalar_size, 20, prk.Z.into_projective());

        // To prove e(witness, element*P_tilde + Q_tilde) == e(accumulated, P_tilde)
        let sigma = E::Fr::rand(rng);
        let rho = E::Fr::rand(rng);
        // Commitment to witness
        // E_C = witness + (sigma + rho) * prk.Z
        let mut E_C = Z_table.multiply(&(sigma + rho));
        E_C.add_assign_mixed(witness);

        // T_sigma = sigma * prk.X
        let T_sigma = X_table.multiply(&sigma);
        // T_rho = rho * prk.Y;
        let T_rho = Y_table.multiply(&rho);
        let delta_sigma = *element * sigma;
        let delta_rho = *element * rho;

        // Commit phase of Schnorr
        // Create blindings for pairing equation
        let r_y = element_blinding.unwrap_or_else(|| E::Fr::rand(rng)); // blinding for proving knowledge of element
        let r_sigma = E::Fr::rand(rng);
        let r_delta_sigma = E::Fr::rand(rng);
        let r_rho = E::Fr::rand(rng);
        let r_delta_rho = E::Fr::rand(rng);

        // R_E = e(E_C, params.P_tilde)^r_y * e(prk.Z, params.P_tilde)^(-r_delta_sigma - r_delta_rho) * e(prk.Z, Q_tilde)^(-r_sigma - r_rho)
        let mut E_C_times_r_y = E_C.clone();
        E_C_times_r_y *= r_y;
        let P_tilde_prepared = E::G2Prepared::from(params.P_tilde);
        let R_E = E::product_of_pairings(
            [
                // e(E_C, params.P_tilde)^r_y = e(r_y * E_C, params.P_tilde)
                (
                    E::G1Prepared::from(E_C_times_r_y.into_affine()),
                    P_tilde_prepared.clone(),
                ),
                // e(prk.Z, params.P_tilde)^(-r_delta_sigma - r_delta_rho) = e((-r_delta_sigma - r_delta_rho) * prk.Z, params.P_tilde)
                (
                    E::G1Prepared::from(
                        Z_table
                            .multiply(&(-r_delta_sigma - r_delta_rho))
                            .into_affine(),
                    ),
                    P_tilde_prepared.clone(),
                ),
                // e(prk.Z, Q_tilde)^(-r_sigma - r_rho) = e((-r_sigma - r_rho) * prk.Z, Q_tilde)
                (
                    E::G1Prepared::from(Z_table.multiply(&(-r_sigma - r_rho)).into_affine()),
                    E::G2Prepared::from(pk.0),
                ),
            ]
            .iter()
            .chain(
                pairing_extra
                    .map_or_else(
                        || {
                            [
                                // To keep both arms of same size. `product_of_pairings` ignores tuples where any element is 0 so the result is not impacted
                                (
                                    E::G1Prepared::from(E::G1Affine::zero()),
                                    E::G2Prepared::from(E::G2Affine::zero()),
                                ),
                            ]
                        },
                        |a| [(E::G1Prepared::from(a), P_tilde_prepared)],
                    )
                    .iter(),
            ),
        );
        // R_sigma = r_sigma * prk.X
        let R_sigma = X_table.multiply(&r_sigma);
        // R_rho = r_rho * prk.Y
        let R_rho = Y_table.multiply(&r_rho);

        // R_delta_sigma = r_y * T_sigma - r_delta_sigma * prk.X
        let mut R_delta_sigma = T_sigma.clone();
        R_delta_sigma *= r_y;
        R_delta_sigma -= X_table.multiply(&r_delta_sigma);

        // R_delta_rho = r_y * T_rho - r_delta_rho * prk.Y;
        let mut R_delta_rho = T_rho.clone();
        R_delta_rho *= r_y;
        R_delta_rho -= Y_table.multiply(&r_delta_rho);
        (
            RandomizedWitness {
                E_C: E_C.into_affine(),
                T_sigma: T_sigma.into_affine(),
                T_rho: T_rho.into_affine(),
            },
            SchnorrCommit {
                R_E,
                R_sigma: R_sigma.into_affine(),
                R_rho: R_rho.into_affine(),
                R_delta_sigma: R_delta_sigma.into_affine(),
                R_delta_rho: R_delta_rho.into_affine(),
            },
            Blindings {
                sigma,
                rho,
                delta_sigma,
                delta_rho,
                r_y,
                r_sigma,
                r_rho,
                r_delta_sigma,
                r_delta_rho,
            },
        )
    }

    /// Contribution to the overall challenge (when using this protocol with others) of this protocol
    fn compute_challenge_contribution<W: Write>(
        randomized_witness: &impl SchnorrChallengeContributor,
        schnorr_commit: &impl SchnorrChallengeContributor,
        accumulator_value: &E::G1Affine,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &ProvingKey<E::G1Affine>,
        mut writer: W,
    ) -> Result<(), VBAccumulatorError> {
        randomized_witness.challenge_contribution(&mut writer)?;
        schnorr_commit.challenge_contribution(&mut writer)?;
        accumulator_value.serialize_unchecked(&mut writer)?;
        pk.0.serialize_unchecked(&mut writer)?;
        params.P.serialize_unchecked(&mut writer)?;
        params.P_tilde.serialize_unchecked(&mut writer)?;
        prk.X.serialize_unchecked(&mut writer)?;
        prk.Y.serialize_unchecked(&mut writer)?;
        prk.Z.serialize_unchecked(&mut writer).map_err(|e| e.into())
    }

    /// Compute responses for the Schnorr protocols
    fn compute_responses(
        element: &E::Fr,
        blindings: &Blindings<E::Fr>,
        challenge: &E::Fr,
    ) -> SchnorrResponse<E::Fr> {
        // Response phase of Schnorr
        let s_y = blindings.r_y + (*challenge * *element);
        let s_sigma = blindings.r_sigma + (*challenge * blindings.sigma);
        let s_rho = blindings.r_rho + (*challenge * blindings.rho);
        let s_delta_sigma = blindings.r_delta_sigma + (*challenge * blindings.delta_sigma);
        let s_delta_rho = blindings.r_delta_rho + (*challenge * blindings.delta_rho);

        SchnorrResponse {
            s_y,
            s_sigma,
            s_rho,
            s_delta_sigma,
            s_delta_rho,
        }
    }

    /// Verifies the (non)membership relation of the randomized witness and (non)member.
    /// `pairing_extra` is used when verifying non-membership proofs and is included in this function
    /// only because its efficient to do a multi-pairing.
    fn verify_proof(
        randomized_witness: &RandomizedWitness<E::G1Affine>,
        schnorr_commit: &SchnorrCommit<E>,
        schnorr_response: &SchnorrResponse<E::Fr>,
        pairing_extra: Option<[E::G1Affine; 2]>,
        accumulator_value: &E::G1Affine,
        challenge: &E::Fr,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &ProvingKey<E::G1Affine>,
    ) -> Result<(), VBAccumulatorError> {
        // There are multiple multiplications with X, Y and Z which can be done in variable time so use wNAF.
        // TODO: Since proving key is fixed, these tables can be created just once and stored.
        let context = WnafContext::new(4);
        let X_table = context.table(prk.X.into_projective());
        let Y_table = context.table(prk.Y.into_projective());
        let Z_table = context.table(prk.Z.into_projective());

        let T_sigma_table = context.table(randomized_witness.T_sigma.into_projective());
        let T_rho_table = context.table(randomized_witness.T_rho.into_projective());
        let E_C_table = context.table(randomized_witness.E_C.into_projective());

        // R_sigma = schnorr_response.s_sigma * prk.X - challenge * randomized_witness.T_sigma
        let mut R_sigma = context
            .mul_with_table(&X_table, &schnorr_response.s_sigma)
            .unwrap();
        R_sigma -= context.mul_with_table(&T_sigma_table, challenge).unwrap();
        if R_sigma.into_affine() != schnorr_commit.R_sigma {
            return Err(VBAccumulatorError::SigmaResponseInvalid);
        }

        // R_rho = schnorr_response.s_rho * prk.Y - challenge * randomized_witness.T_rho;
        let mut R_rho = context
            .mul_with_table(&Y_table, &schnorr_response.s_rho)
            .unwrap();
        R_rho -= context.mul_with_table(&T_rho_table, challenge).unwrap();
        if R_rho.into_affine() != schnorr_commit.R_rho {
            return Err(VBAccumulatorError::RhoResponseInvalid);
        }

        // R_delta_sigma = schnorr_response.s_y * randomized_witness.T_sigma - schnorr_response.s_delta_sigma * prk.X;
        let mut R_delta_sigma = context
            .mul_with_table(&T_sigma_table, &schnorr_response.s_y)
            .unwrap();
        R_delta_sigma -= context
            .mul_with_table(&X_table, &schnorr_response.s_delta_sigma)
            .unwrap();
        if R_delta_sigma.into_affine() != schnorr_commit.R_delta_sigma {
            return Err(VBAccumulatorError::DeltaSigmaResponseInvalid);
        }

        // R_delta_rho = schnorr_response.s_y * randomized_witness.T_rho - schnorr_response.s_delta_rho * prk.Y;
        let mut R_delta_rho = context
            .mul_with_table(&T_rho_table, &schnorr_response.s_y)
            .unwrap();
        R_delta_rho -= context
            .mul_with_table(&Y_table, &schnorr_response.s_delta_rho)
            .unwrap();
        if R_delta_rho.into_affine() != schnorr_commit.R_delta_rho {
            return Err(VBAccumulatorError::DeltaRhoResponseInvalid);
        }

        let P_tilde_prepared = E::G2Prepared::from(params.P_tilde);
        let Q_tilde_prepared = E::G2Prepared::from(pk.0);

        let R_E = E::product_of_pairings(
            [
                // e(E_C, params.P_tilde)^s_y = e(s_y * E_C, params.P_tilde)
                (
                    E::G1Prepared::from(
                        context
                            .mul_with_table(&E_C_table, &schnorr_response.s_y)
                            .unwrap()
                            .into_affine(),
                    ),
                    P_tilde_prepared.clone(),
                ),
                // e(Z, params.P_tilde)^(s_delta_sigma - s_delta_rho) = e((s_delta_sigma - s_delta_rho) * Z, params.P_tilde)
                (
                    E::G1Prepared::from(
                        context
                            .mul_with_table(
                                &Z_table,
                                &(-schnorr_response.s_delta_sigma - schnorr_response.s_delta_rho),
                            )
                            .unwrap()
                            .into_affine(),
                    ),
                    P_tilde_prepared.clone(),
                ),
                // e(Z, Q_tilde)^(s_sigma - s_rho) = e((s_sigma - s_rho) * Z, Q_tilde)
                (
                    E::G1Prepared::from(
                        context
                            .mul_with_table(
                                &Z_table,
                                &(-schnorr_response.s_sigma - schnorr_response.s_rho),
                            )
                            .unwrap()
                            .into_affine(),
                    ),
                    Q_tilde_prepared.clone(),
                ),
                // e(V, params.P_tilde)^-challenge = e(-challenge * V, params.P_tilde)
                (
                    E::G1Prepared::from(
                        accumulator_value
                            .mul((-*challenge).into_repr())
                            .into_affine(),
                    ),
                    P_tilde_prepared.clone(),
                ),
                // e(E_C, Q_tilde)^challenge = e(challenge * E_C, Q_tilde)
                (
                    E::G1Prepared::from(
                        context
                            .mul_with_table(&E_C_table, challenge)
                            .unwrap()
                            .into_affine(),
                    ),
                    Q_tilde_prepared,
                ),
            ]
            .iter()
            .chain(
                pairing_extra
                    .map_or_else(
                        || {
                            [
                                // To keep both arms of same size. `product_of_pairings` ignores tuples where any element is 0 so the result is not impacted
                                (
                                    E::G1Prepared::from(E::G1Affine::zero()),
                                    E::G2Prepared::from(E::G2Affine::zero()),
                                ),
                                (
                                    E::G1Prepared::from(E::G1Affine::zero()),
                                    E::G2Prepared::from(E::G2Affine::zero()),
                                ),
                            ]
                        },
                        |[a, b]| {
                            [
                                (E::G1Prepared::from(a), P_tilde_prepared.clone()),
                                (E::G1Prepared::from(b), P_tilde_prepared),
                            ]
                        },
                    )
                    .iter(),
            ),
        );

        if R_E != schnorr_commit.R_E {
            return Err(VBAccumulatorError::PairingResponseInvalid);
        }

        Ok(())
    }
}

impl<E> ProofProtocol<E> for MembershipProofProtocol<E> where E: PairingEngine {}

impl<E> MembershipProofProtocol<E>
where
    E: PairingEngine,
{
    /// Initialize a membership proof protocol. Delegates to [`randomize_witness_and_compute_commitments`]
    ///
    /// [`randomize_witness_and_compute_commitments`]: ProofProtocol::randomize_witness_and_compute_commitments
    pub fn init<R: RngCore>(
        rng: &mut R,
        element: &E::Fr,
        element_blinding: Option<E::Fr>,
        witness: &MembershipWitness<E::G1Affine>,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &MembershipProvingKey<E::G1Affine>,
    ) -> Self {
        let (rw, sc, bl) = Self::randomize_witness_and_compute_commitments(
            rng,
            &element,
            element_blinding,
            &witness.0,
            None,
            pk,
            params,
            &prk.0,
        );
        Self {
            element: element.clone(),
            randomized_witness: MembershipRandomizedWitness(rw),
            schnorr_commit: MembershipSchnorrCommit(sc),
            schnorr_blindings: MembershipBlindings(bl),
        }
    }

    /// Contribution of this protocol to the overall challenge (when using this protocol as a sub-protocol).
    /// Delegates to [`compute_challenge_contribution`]
    ///
    /// [`compute_challenge_contribution`]: ProofProtocol::compute_challenge_contribution
    pub fn challenge_contribution<W: Write>(
        &self,
        accumulator_value: &E::G1Affine,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &MembershipProvingKey<E::G1Affine>,
        writer: W,
    ) -> Result<(), VBAccumulatorError> {
        Self::compute_challenge_contribution(
            &self.randomized_witness,
            &self.schnorr_commit,
            accumulator_value,
            pk,
            params,
            &prk.0,
            writer,
        )
    }

    /// Create membership proof once the overall challenge is ready. Delegates to [`compute_responses`]
    ///
    /// [`compute_responses`]: ProofProtocol::compute_responses
    pub fn gen_proof(self, challenge: &E::Fr) -> MembershipProof<E> {
        let resp = Self::compute_responses(&self.element, &self.schnorr_blindings.0, challenge);
        MembershipProof {
            randomized_witness: self.randomized_witness,
            schnorr_commit: self.schnorr_commit,
            schnorr_response: MembershipSchnorrResponse(resp),
        }
    }
}

impl<E> ProofProtocol<E> for NonMembershipProofProtocol<E> where E: PairingEngine {}

impl<E> NonMembershipProofProtocol<E>
where
    E: PairingEngine,
{
    /// Initialize a non-membership proof protocol. Create blindings for proving `witness.d != 0` and
    /// then delegates to [`randomize_witness_and_compute_commitments`]
    ///
    /// [`randomize_witness_and_compute_commitments`]: ProofProtocol::randomize_witness_and_compute_commitments
    pub fn init<R: RngCore>(
        rng: &mut R,
        element: &E::Fr,
        element_blinding: Option<E::Fr>,
        witness: &NonMembershipWitness<E::G1Affine>,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &NonMembershipProvingKey<E::G1Affine>,
    ) -> Self {
        // TODO: Since proving key is fixed, these tables can be created just once and stored.
        // There are multiple multiplications with P and K so create tables for them. 20 multiplications
        //         // is the upper bound
        let scalar_size = E::Fr::size_in_bits();
        let P_table = WindowTable::new(scalar_size, 20, params.P.into_projective());
        let K_table = WindowTable::new(scalar_size, 20, prk.K.into_projective());

        // To prove non-zero d of witness
        let tau = E::Fr::rand(rng); // blinding in commitment to d
        let pi = E::Fr::rand(rng);

        // Commitment to d
        // E_d = witness.d * pk.P + tau * prk.K
        let mut E_d = P_table.multiply(&witness.d);
        E_d += K_table.multiply(&tau);

        // Commitment to d^-1
        // E_d_inv = 1/witness.d * pk.P + pi * prk.K;
        let mut E_d_inv = P_table.multiply(&witness.d.inverse().unwrap());
        E_d_inv += K_table.multiply(&pi);

        // Create blindings for d != 0
        let r_u = E::Fr::rand(rng); // blinding for proving knowledge of d
        let r_v = E::Fr::rand(rng); // blinding for proving knowledge of tau
        let r_w = E::Fr::rand(rng);

        // R_A = r_u * pk.P + r_v * prk.K;
        let mut R_A = P_table.multiply(&r_u);
        R_A += K_table.multiply(&r_v);

        // R_B = r_u * E_d_inv + r_w * prk.K;
        let mut R_B = E_d_inv.clone();
        R_B *= r_u;
        R_B += K_table.multiply(&r_w);

        // new R_E = e(E_C, params.P_tilde)^r_y * e(prk.Z, params.P_tilde)^(-r_delta_sigma - r_delta_rho) * e(prk.Z, Q_tilde)^(-r_sigma - r_rho) * e(prk.K, params.P_tilde)^-r_v
        // sc.R_E = e(E_C, params.P_tilde)^r_y * e(prk.Z, params.P_tilde)^(-r_delta_sigma - r_delta_rho) * e(prk.Z, Q_tilde)^(-r_sigma - r_rho)
        // => new R_E = e(prk.K, params.P_tilde)^-r_v * sc.R_E = e(-r_v * prk.K, params.P_tilde) * sc.R_E
        let (rw, sc, bl) = Self::randomize_witness_and_compute_commitments(
            rng,
            &element,
            element_blinding,
            &witness.C,
            Some(K_table.multiply(&-r_v).into_affine()),
            pk,
            params,
            &prk.XYZ,
        );

        Self {
            element: element.clone(),
            d: witness.d.clone(),
            randomized_witness: NonMembershipRandomizedWitness {
                C: rw,
                E_d: E_d.into_affine(),
                E_d_inv: E_d_inv.into_affine(),
            },
            schnorr_commit: NonMembershipSchnorrCommit {
                C: sc,
                R_A: R_A.into_affine(),
                R_B: R_B.into_affine(),
            },
            schnorr_blindings: NonMembershipBlindings {
                C: bl,
                tau,
                pi,
                r_u,
                r_v,
                r_w,
            },
        }
    }

    /// Contribution of this protocol to the overall challenge (when using this protocol as a sub-protocol).
    /// Delegates to [`compute_challenge_contribution`]
    ///
    /// [`compute_challenge_contribution`]: ProofProtocol::compute_challenge_contribution
    pub fn challenge_contribution<W: Write>(
        &self,
        accumulator_value: &E::G1Affine,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &NonMembershipProvingKey<E::G1Affine>,
        mut writer: W,
    ) -> Result<(), VBAccumulatorError> {
        Self::compute_challenge_contribution(
            &self.randomized_witness,
            &self.schnorr_commit,
            accumulator_value,
            pk,
            params,
            &prk.XYZ,
            &mut writer,
        )?;
        prk.K.serialize_unchecked(&mut writer).map_err(|e| e.into())
    }

    /// Create membership proof once the overall challenge is ready. Computes the response for `witness.d`
    /// and then delegates to [`compute_responses`]
    ///
    /// [`compute_responses`]: ProofProtocol::compute_responses
    pub fn gen_proof(self, challenge: &E::Fr) -> NonMembershipProof<E> {
        // For d != 0
        let challenge_times_d = *challenge * self.d;
        let s_u = self.schnorr_blindings.r_u + challenge_times_d;
        let s_v = self.schnorr_blindings.r_v + (*challenge * self.schnorr_blindings.tau);
        let s_w = self.schnorr_blindings.r_w - (challenge_times_d * self.schnorr_blindings.pi);

        let resp = Self::compute_responses(&self.element, &self.schnorr_blindings.C, challenge);

        NonMembershipProof {
            randomized_witness: self.randomized_witness,
            schnorr_commit: self.schnorr_commit,
            schnorr_response: NonMembershipSchnorrResponse {
                C: resp,
                s_u,
                s_v,
                s_w,
            },
        }
    }
}

impl<E> MembershipProof<E>
where
    E: PairingEngine,
{
    /// Challenge contribution for this proof
    pub fn challenge_contribution<W: Write>(
        &self,
        accumulator_value: &E::G1Affine,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &MembershipProvingKey<E::G1Affine>,
        writer: W,
    ) -> Result<(), VBAccumulatorError> {
        MembershipProofProtocol::compute_challenge_contribution(
            &self.randomized_witness,
            &self.schnorr_commit,
            accumulator_value,
            pk,
            params,
            &prk.0,
            writer,
        )
    }

    /// Verify this proof. Delegates to [`verify_proof`]
    ///
    /// [`verify_proof`]: ProofProtocol::verify_proof
    pub fn verify(
        &self,
        accumulator_value: &E::G1Affine,
        challenge: &E::Fr,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &MembershipProvingKey<E::G1Affine>,
    ) -> Result<(), VBAccumulatorError> {
        <MembershipProofProtocol<E> as ProofProtocol<E>>::verify_proof(
            &self.randomized_witness.0,
            &self.schnorr_commit.0,
            &self.schnorr_response.0,
            None,
            accumulator_value,
            challenge,
            pk,
            params,
            &prk.0,
        )
    }

    /// Get response for Schnorr protocol for the member. This is useful when the member is also used
    /// in another relation that is proven along this protocol.
    pub fn get_schnorr_response_for_element(&self) -> &E::Fr {
        self.schnorr_response.0.get_response_for_element()
    }
}

impl<E> NonMembershipProof<E>
where
    E: PairingEngine,
{
    /// Challenge contribution for this proof
    pub fn challenge_contribution<W: Write>(
        &self,
        accumulator_value: &E::G1Affine,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &NonMembershipProvingKey<E::G1Affine>,
        mut writer: W,
    ) -> Result<(), VBAccumulatorError> {
        NonMembershipProofProtocol::compute_challenge_contribution(
            &self.randomized_witness,
            &self.schnorr_commit,
            accumulator_value,
            pk,
            params,
            &prk.XYZ,
            &mut writer,
        )?;
        prk.K.serialize_unchecked(&mut writer).map_err(|e| e.into())
    }

    /// Verify this proof. Verify the responses for the relation `witness.d != 0` and then delegates
    /// to [`verify_proof`]
    ///
    /// [`verify_proof`]: ProofProtocol::verify_proof
    pub fn verify(
        &self,
        accumulator_value: &E::G1Affine,
        challenge: &E::Fr,
        pk: &PublicKey<E::G2Affine>,
        params: &SetupParams<E>,
        prk: &NonMembershipProvingKey<E::G1Affine>,
    ) -> Result<(), VBAccumulatorError> {
        // There are multiple multiplications with K, P and E_d which can be done in variable time so use wNAF.
        let context = WnafContext::new(4);
        let K_table = context.table(prk.K.into_projective());
        let P_table = context.table(params.P.into_projective());
        let E_d_table = context.table(self.randomized_witness.E_d.into_projective());

        // R_A = schnorr_response.s_u * params.P + schnorr_response.s_v * prk.K - challenge * randomized_witness.E_d;
        let mut R_A = context
            .mul_with_table(&P_table, &self.schnorr_response.s_u)
            .unwrap();
        R_A += context
            .mul_with_table(&K_table, &self.schnorr_response.s_v)
            .unwrap();
        R_A -= context.mul_with_table(&E_d_table, challenge).unwrap();

        if R_A.into_affine() != self.schnorr_commit.R_A {
            return Err(VBAccumulatorError::E_d_ResponseInvalid);
        }

        // R_B = schnorr_response.s_w * prk.K + schnorr_response.s_u * randomized_witness.E_d_inv - challenge * params.P;
        let mut R_B = context
            .mul_with_table(&K_table, &self.schnorr_response.s_w)
            .unwrap();
        R_B += self
            .randomized_witness
            .E_d_inv
            .mul(self.schnorr_response.s_u.into_repr());
        R_B -= context.mul_with_table(&P_table, challenge).unwrap();

        if R_B.into_affine() != self.schnorr_commit.R_B {
            return Err(VBAccumulatorError::E_d_inv_ResponseInvalid);
        }

        let pairing_contribution = [
            // -schnorr_response.s_v * prk.K
            context
                .mul_with_table(&K_table, &-self.schnorr_response.s_v)
                .unwrap()
                .into_affine(),
            // challenge * randomized_witness.E_d
            context
                .mul_with_table(&E_d_table, challenge)
                .unwrap()
                .into_affine(),
        ];
        <NonMembershipProofProtocol<E> as ProofProtocol<E>>::verify_proof(
            &self.randomized_witness.C,
            &self.schnorr_commit.C,
            &self.schnorr_response.C,
            Some(pairing_contribution),
            accumulator_value,
            challenge,
            pk,
            params,
            &prk.XYZ,
        )
    }

    /// Get response for Schnorr protocol for the non-member. This is useful when the non-member is also used
    /// in another relation that is proven along this protocol.
    pub fn get_schnorr_response_for_element(&self) -> &E::Fr {
        self.schnorr_response.C.get_response_for_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::positive::{tests::setup_positive_accum, Accumulator};
    use crate::test_serialization;
    use crate::universal::tests::setup_universal_accum;

    use ark_bls12_381::Bls12_381;
    use ark_std::{rand::rngs::StdRng, rand::SeedableRng, UniformRand};
    use blake2::Blake2b;
    use schnorr_pok::compute_random_oracle_challenge;
    use std::time::{Duration, Instant};

    type Fr = <Bls12_381 as PairingEngine>::Fr;

    #[test]
    fn membership_proof_positive_accumulator() {
        // Proof of knowledge of membership witness
        let mut rng = StdRng::seed_from_u64(0u64);

        let (params, keypair, mut accumulator, mut state) = setup_positive_accum(&mut rng);
        let prk = MembershipProvingKey::generate_using_rng(&mut rng);

        test_serialization!(
            MembershipProvingKey<<Bls12_381 as PairingEngine>::G1Affine>,
            prk
        );

        let mut elems = vec![];
        let mut witnesses = vec![];
        let count = 10;

        for _ in 0..count {
            let elem = Fr::rand(&mut rng);
            accumulator = accumulator
                .add(elem.clone(), &keypair.secret_key, &mut state)
                .unwrap();
            elems.push(elem);
        }

        for i in 0..count {
            let w = accumulator
                .get_membership_witness(&elems[i], &keypair.secret_key, &state)
                .unwrap();
            assert!(accumulator.verify_membership(&elems[i], &w, &keypair.public_key, &params));
            witnesses.push(w);
        }

        let mut proof_create_duration = Duration::default();
        let mut proof_verif_duration = Duration::default();

        for i in 0..count {
            let start = Instant::now();
            let protocol = MembershipProofProtocol::init(
                &mut rng,
                &elems[i],
                None,
                &witnesses[i],
                &keypair.public_key,
                &params,
                &prk,
            );
            proof_create_duration += start.elapsed();

            test_serialization!(MembershipProofProtocol<Bls12_381>, protocol);

            let mut chal_bytes_prover = vec![];
            protocol
                .challenge_contribution(
                    accumulator.value(),
                    &keypair.public_key,
                    &params,
                    &prk,
                    &mut chal_bytes_prover,
                )
                .unwrap();
            let challenge_prover =
                compute_random_oracle_challenge::<Fr, Blake2b>(&chal_bytes_prover);

            let start = Instant::now();
            let proof = protocol.gen_proof(&challenge_prover);
            proof_create_duration += start.elapsed();

            // Proof can be serialized
            test_serialization!(MembershipProof<Bls12_381>, proof);

            let mut chal_bytes_verifier = vec![];
            proof
                .challenge_contribution(
                    accumulator.value(),
                    &keypair.public_key,
                    &params,
                    &prk,
                    &mut chal_bytes_verifier,
                )
                .unwrap();
            let challenge_verifier =
                compute_random_oracle_challenge::<Fr, Blake2b>(&chal_bytes_verifier);

            assert_eq!(challenge_prover, challenge_verifier);

            let start = Instant::now();
            proof
                .verify(
                    &accumulator.value(),
                    &challenge_verifier,
                    &keypair.public_key,
                    &params,
                    &prk,
                )
                .unwrap();
            proof_verif_duration += start.elapsed();
        }

        println!(
            "Time to create {} membership proofs is {:?}",
            count, proof_create_duration
        );
        println!(
            "Time to verify {} membership proofs is {:?}",
            count, proof_verif_duration
        );
    }

    #[test]
    fn non_membership_proof_universal_accumulator() {
        // Proof of knowledge of non-membership witness
        let max = 100;
        let mut rng = StdRng::seed_from_u64(0u64);

        let (params, keypair, mut accumulator, initial_elems, mut state) =
            setup_universal_accum(&mut rng, max);
        let prk = NonMembershipProvingKey::generate_using_rng(&mut rng);

        test_serialization!(
            NonMembershipProvingKey<<Bls12_381 as PairingEngine>::G1Affine>,
            prk
        );

        let mut elems = vec![];
        let mut witnesses = vec![];
        let count = 10;

        for _ in 0..50 {
            accumulator = accumulator
                .add(
                    Fr::rand(&mut rng),
                    &keypair.secret_key,
                    &initial_elems,
                    &mut state,
                )
                .unwrap();
        }

        for _ in 0..count {
            let elem = Fr::rand(&mut rng);
            let w = accumulator
                .get_non_membership_witness(&elem, &keypair.secret_key, &mut state, &params)
                .unwrap();
            assert!(accumulator.verify_non_membership(&elem, &w, &keypair.public_key, &params));
            elems.push(elem);
            witnesses.push(w);
        }

        let mut proof_create_duration = Duration::default();
        let mut proof_verif_duration = Duration::default();

        for i in 0..count {
            let start = Instant::now();
            let protocol = NonMembershipProofProtocol::init(
                &mut rng,
                &elems[i],
                None,
                &witnesses[i],
                &keypair.public_key,
                &params,
                &prk,
            );
            proof_create_duration += start.elapsed();

            test_serialization!(NonMembershipProofProtocol<Bls12_381>, protocol);

            let mut chal_bytes_prover = vec![];
            protocol
                .challenge_contribution(
                    accumulator.value(),
                    &keypair.public_key,
                    &params,
                    &prk,
                    &mut chal_bytes_prover,
                )
                .unwrap();
            let challenge_prover =
                compute_random_oracle_challenge::<Fr, Blake2b>(&chal_bytes_prover);

            let start = Instant::now();
            let proof = protocol.gen_proof(&challenge_prover);
            proof_create_duration += start.elapsed();

            let mut chal_bytes_verifier = vec![];
            proof
                .challenge_contribution(
                    accumulator.value(),
                    &keypair.public_key,
                    &params,
                    &prk,
                    &mut chal_bytes_verifier,
                )
                .unwrap();
            let challenge_verifier =
                compute_random_oracle_challenge::<Fr, Blake2b>(&chal_bytes_verifier);

            assert_eq!(challenge_prover, challenge_verifier);

            test_serialization!(NonMembershipProof<Bls12_381>, proof);

            let start = Instant::now();
            proof
                .verify(
                    &accumulator.value(),
                    &challenge_verifier,
                    &keypair.public_key,
                    &params,
                    &prk,
                )
                .unwrap();
            proof_verif_duration += start.elapsed();
        }

        println!(
            "Time to create {} non-membership proofs is {:?}",
            count, proof_create_duration
        );
        println!(
            "Time to verify {} non-membership proofs is {:?}",
            count, proof_verif_duration
        );
    }
}
