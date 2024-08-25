//! Generate a pseudonym. This is generated by following the protocol described in section 4 of the paper.
//!
//! Notation
//! ```docs
//! T = e(Z, usk_hat)
//! A = e(Z, W_hat)
//! B = e(Z, C2_hat) / T
//! E = e(C2, g_hat)e(g^-1, C2_hat)
//! F = e(W, g_hat)
//! G = e(g^-1, W_hat)
//! H = e(C2, ivk_hat)e(g^-1, g_hat)
//! I = e(W, ivk)
//! J = e(C2, g_hat^-1)
//! ```
//!
//! Following is a description of the protocol to prove `E = F^{β}.G^{α} ∧ H = I^{β}.F^{β.s}.J^s`. This is only part of the protocol but the rest is straightforward (implemention is of the full protocol).
//! Follows the notation of the paper and thus uses the multiplicative notation.
//!
//! The user creates `K_1 = F^s.G^r_1` and `K_2 = F^{β.s}.G^r_2` and uses the protocol for multiplicative relation among `K_1`, `K_2` and `E`
//! to prove that the exponents of `F` in `K_2` is the product of exponents of `F` in `K_1` and `E` while also proving the equality of those exponents
//! with the ones in `H`. Following is the detailed protocol with P and V denoting the prover and verifier respectively.
//!
//! 1. P chooses random `r_1, r_2` and computes `K_1 = F^s.G^r_1` and `K_2 = F^{β.s}.G^r_2`. Now `K_2 = E^s.G^{r_2 - α.s}`. Let `r3 = r2 - α*s`
//! 2. Now P starts executing the Schnorr protocol. It chooses random θ_1, θ_2, θ_3, R_1, R_2, R_3, R_4.
//! 3. P computes
//!     T_1 = F^{θ_1}.G^{R_1} (for K_1)
//!     T_2 = F^{θ_2}.G^{R_2} (for E)
//!     T_3 = F^{θ_3}.G^{R_3} (for K_2)
//!     T_4 = E^{θ_1}.G^{R_4} (for K_2)   
//!     T_5 = I^{θ_2}.F^{θ_3}.J^{θ_1}  (for H)
//! 4. V gives the challenge c.
//! 5. P computes the following (s_i, t_i) and sends to V along with T_i.
//!     s_1 = θ_1 + c.s
//!     s_2 = θ_2 + c.β
//!     s_3 = θ_3 + c.β.s
//!     t_1 = R_1 + c.r_1
//!     t_2 = R_2 + c.α
//!     t_3 = R_3 + c.r_2
//!     t_4 = R_4 + c(r_2 - α.s)
//! 6. V checks the following
//!     T_1 == K_1^-c.F^{s_1}.G^{t_1}
//!     T_2 == E^-c.F^{s_2}.G^{t_2}
//!     T_3 == K_2^-c.F^{s_3}.G^{t_3}
//!     T_4 == K_2^-c.E^{s_1}.G^{t_4}
//!     T_5 == H^-c.I^{s_2}.F^{s_3}.J^{s_1}
//!
//! The implementation uses precomputation but if pre-computation is not done then some of these can utilize multi-pairings
//! which are more efficient. But using precomputation is faster.

use crate::{
    error::SyraError,
    setup::{PreparedIssuerPublicKey, PreparedSetupParams, PreparedUserSecretKey},
};
use ark_ec::{
    pairing::{Pairing, PairingOutput},
    AffineRepr, CurveGroup,
};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::{io::Write, mem, ops::Neg, rand::RngCore, vec::Vec, UniformRand};
use dock_crypto_utils::elgamal::Ciphertext as ElgamalCiphertext;
use serde_with::serde_as;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Protocol to generate a pseudonym and the proof of correctness of it.
#[derive(
    Clone, PartialEq, Eq, Debug, Zeroize, ZeroizeOnDrop, CanonicalSerialize, CanonicalDeserialize,
)]
pub struct PseudonymGenProtocol<E: Pairing> {
    alpha: E::ScalarField,
    beta: E::ScalarField,
    s: E::ScalarField,
    beta_times_s: E::ScalarField,
    r1: E::ScalarField,
    r2: E::ScalarField,
    r3: E::ScalarField,
    /// Pseudonym
    #[zeroize(skip)]
    pub T: PairingOutput<E>,
    #[zeroize(skip)]
    pub C: ElgamalCiphertext<E::G1Affine>,
    #[zeroize(skip)]
    pub C_hat: ElgamalCiphertext<E::G2Affine>,
    #[zeroize(skip)]
    pub K1: PairingOutput<E>,
    #[zeroize(skip)]
    pub K2: PairingOutput<E>,
    blinding_alpha: E::ScalarField,
    blinding_beta: E::ScalarField,
    blinding_s: E::ScalarField,
    blinding_beta_times_s: E::ScalarField,
    blinding_r1: E::ScalarField,
    blinding_r2: E::ScalarField,
    blinding_r3: E::ScalarField,
    pub t_C1: E::G1Affine,
    pub t_C1_hat: E::G2Affine,
    pub t_B: PairingOutput<E>,
    pub t_E: PairingOutput<E>,
    pub t_H: PairingOutput<E>,
    pub t_K1: PairingOutput<E>,
    pub t_K2: PairingOutput<E>,
    pub t_K2_product: PairingOutput<E>,
}

/// This contains the pseudonym as well its proof of correctness
#[serde_as]
#[derive(Clone, PartialEq, Eq, Debug, CanonicalSerialize, CanonicalDeserialize)]
pub struct PseudonymProof<E: Pairing> {
    /// Pseudonym
    pub T: PairingOutput<E>,
    pub C: ElgamalCiphertext<E::G1Affine>,
    pub C_hat: ElgamalCiphertext<E::G2Affine>,
    pub K1: PairingOutput<E>,
    pub K2: PairingOutput<E>,
    pub t_C1: E::G1Affine,
    pub t_C1_hat: E::G2Affine,
    pub t_B: PairingOutput<E>,
    pub t_E: PairingOutput<E>,
    pub t_H: PairingOutput<E>,
    pub t_K1: PairingOutput<E>,
    pub t_K2: PairingOutput<E>,
    pub t_K2_product: PairingOutput<E>,
    pub resp_alpha: E::ScalarField,
    pub resp_beta: E::ScalarField,
    pub resp_s: E::ScalarField,
    pub resp_beta_times_s: E::ScalarField,
    pub resp_r1: E::ScalarField,
    pub resp_r2: E::ScalarField,
    pub resp_r3: E::ScalarField,
}

impl<E: Pairing> PseudonymGenProtocol<E> {
    /// `Z` is the context ctx mapped (hashed) to a group element
    /// `s` is the user-id which was the message in the VRF and `blinding` is the randomness used for `s` in the Schnorr protocol.
    /// This will be set by the caller when this is used in conjunction with another Schnorr protocol and `s` has to be
    /// proved equal to the witness.
    pub fn init<R: RngCore>(
        rng: &mut R,
        Z: E::G1Affine,
        s: E::ScalarField,
        blinding: Option<E::ScalarField>,
        user_sk: impl Into<PreparedUserSecretKey<E>>,
        issuer_pk: PreparedIssuerPublicKey<E>,
        params: impl Into<PreparedSetupParams<E>>,
    ) -> Self {
        let user_sk = user_sk.into();
        let params = params.into();
        let alpha = E::ScalarField::rand(rng);
        let beta = E::ScalarField::rand(rng);
        let beta_times_s = beta * s;
        let T = E::pairing(E::G1Prepared::from(&Z), user_sk.1.clone());
        let C = ElgamalCiphertext::<E::G1Affine>::new_given_randomness(
            &user_sk.0 .0,
            &beta,
            &issuer_pk.w,
            &params.g,
        );
        let C_hat = ElgamalCiphertext::<E::G2Affine>::new_given_randomness(
            &user_sk.0 .1,
            &alpha,
            &issuer_pk.w_hat,
            &params.g_hat,
        );
        let z_prepared = E::G1Prepared::from(Z);
        let C2_prepared = E::G1Prepared::from(C.encrypted);
        let C2_hat_prepared = E::G2Prepared::from(C_hat.encrypted);
        let A = E::pairing(z_prepared.clone(), issuer_pk.w_hat_prepared.clone());
        let E = E::pairing(C2_prepared.clone(), params.g_hat_prepared.clone())
            - E::pairing(E::G1Prepared::from(params.g), C2_hat_prepared);
        // e(C2, -g_hat) = e(-C2, g_hat)
        let J = E::pairing(
            E::G1Prepared::from(C.encrypted.into_group().neg()),
            params.g_hat_prepared,
        );
        // F, G and I are part of the precomputed public params
        let F = issuer_pk.w_g_hat;
        let G = issuer_pk.minus_g_w_hat;
        let I = issuer_pk.w_vk;

        let r1 = E::ScalarField::rand(rng);
        let r2 = E::ScalarField::rand(rng);
        let r3 = r2 - (alpha * s);
        // K1 = F*s + G*r1
        let K1 = F * s + G * r1;
        // K2 = F * {β.s} + G*r2 = E*s + G*{r2 - α.s}
        let K2 = F * beta_times_s + G * r2;

        // Create blindings for first phase of the Schnorr protocol
        let blinding_alpha = E::ScalarField::rand(rng);
        let blinding_beta = E::ScalarField::rand(rng);
        let blinding_s = blinding.unwrap_or_else(|| E::ScalarField::rand(rng));
        let blinding_beta_times_s = E::ScalarField::rand(rng);
        let blinding_r1 = E::ScalarField::rand(rng);
        let blinding_r2 = E::ScalarField::rand(rng);
        let blinding_r3 = E::ScalarField::rand(rng);

        // Commit to those blindings
        let t_C1 = (params.g * blinding_beta).into_affine();
        let t_C1_hat = (params.g_hat * blinding_alpha).into_affine();
        let t_B = A * blinding_alpha;
        let t_E = F * blinding_beta + G * blinding_alpha;
        let F_bs = F * blinding_beta_times_s;
        let t_H = I * blinding_beta + F_bs + J * blinding_s;
        let t_K1 = F * blinding_s + G * blinding_r1;
        let t_K2 = F_bs + G * blinding_r2;
        let t_K2_product = E * blinding_s + G * blinding_r3;
        Self {
            T,
            C,
            C_hat,
            K1,
            K2,
            alpha,
            beta,
            s,
            beta_times_s,
            r1,
            r2,
            r3,
            blinding_alpha,
            blinding_beta,
            blinding_s,
            blinding_beta_times_s,
            blinding_r1,
            blinding_r2,
            blinding_r3,
            t_C1,
            t_C1_hat,
            t_B,
            t_E,
            t_H,
            t_K1,
            t_K2,
            t_K2_product,
        }
    }

    pub fn challenge_contribution<W: Write>(
        &self,
        Z: &E::G1Affine,
        writer: W,
    ) -> Result<(), SyraError> {
        Self::compute_challenge_contribution(
            Z,
            &self.T,
            &self.C,
            &self.C_hat,
            &self.K1,
            &self.K2,
            &self.t_C1,
            &self.t_C1_hat,
            &self.t_B,
            &self.t_E,
            &self.t_H,
            &self.t_K1,
            &self.t_K2,
            &self.t_K2_product,
            writer,
        )
    }

    pub fn gen_proof(mut self, challenge: &E::ScalarField) -> PseudonymProof<E> {
        // Create responses for final phase of the Schnorr protocol
        let resp_beta = self.blinding_beta + self.beta * challenge;
        let resp_alpha = self.blinding_alpha + self.alpha * challenge;
        let resp_s = self.blinding_s + self.s * challenge;
        let resp_beta_times_s = self.blinding_beta_times_s + self.beta_times_s * challenge;
        let resp_r1 = self.blinding_r1 + self.r1 * challenge;
        let resp_r2 = self.blinding_r2 + self.r2 * challenge;
        let resp_r3 = self.blinding_r3 + self.r3 * challenge;
        PseudonymProof {
            T: self.T,
            C: mem::take(&mut self.C),
            C_hat: mem::take(&mut self.C_hat),
            K1: self.K1,
            K2: self.K2,
            t_C1: self.t_C1,
            t_C1_hat: self.t_C1_hat,
            t_B: self.t_B,
            t_E: self.t_E,
            t_H: self.t_H,
            t_K1: self.t_K1,
            t_K2: self.t_K2,
            t_K2_product: self.t_K2_product,
            resp_beta,
            resp_alpha,
            resp_s,
            resp_beta_times_s,
            resp_r1,
            resp_r2,
            resp_r3,
        }
    }

    pub fn compute_challenge_contribution<W: Write>(
        Z: &E::G1Affine,
        T: &PairingOutput<E>,
        C: &ElgamalCiphertext<E::G1Affine>,
        C_hat: &ElgamalCiphertext<E::G2Affine>,
        K1: &PairingOutput<E>,
        K2: &PairingOutput<E>,
        t_C1: &E::G1Affine,
        t_C1_hat: &E::G2Affine,
        t_B: &PairingOutput<E>,
        t_E: &PairingOutput<E>,
        t_H: &PairingOutput<E>,
        t_K1: &PairingOutput<E>,
        t_K2: &PairingOutput<E>,
        t_K2_product: &PairingOutput<E>,
        mut writer: W,
    ) -> Result<(), SyraError> {
        Z.serialize_compressed(&mut writer)?;
        T.serialize_compressed(&mut writer)?;
        C.serialize_compressed(&mut writer)?;
        C_hat.serialize_compressed(&mut writer)?;
        K1.serialize_compressed(&mut writer)?;
        K2.serialize_compressed(&mut writer)?;
        t_C1.serialize_compressed(&mut writer)?;
        t_C1_hat.serialize_compressed(&mut writer)?;
        t_B.serialize_compressed(&mut writer)?;
        t_E.serialize_compressed(&mut writer)?;
        t_H.serialize_compressed(&mut writer)?;
        t_K1.serialize_compressed(&mut writer)?;
        t_K2.serialize_compressed(&mut writer)?;
        t_K2_product.serialize_compressed(&mut writer)?;
        Ok(())
    }
}

impl<E: Pairing> PseudonymProof<E> {
    pub fn verify(
        &self,
        challenge: &E::ScalarField,
        Z: E::G1Affine,
        issuer_pk: PreparedIssuerPublicKey<E>,
        params: impl Into<PreparedSetupParams<E>>,
    ) -> Result<(), SyraError> {
        let params = params.into();
        let z_prepared = E::G1Prepared::from(Z);
        let C2_prepared = E::G1Prepared::from(self.C.encrypted);
        let C2_hat_prepared = E::G2Prepared::from(self.C_hat.encrypted);
        let A = E::pairing(z_prepared.clone(), issuer_pk.w_hat_prepared.clone());
        let B = E::pairing(z_prepared.clone(), C2_hat_prepared.clone()) - self.T;
        let E = E::pairing(C2_prepared.clone(), params.g_hat_prepared.clone())
            - E::pairing(E::G1Prepared::from(params.g), C2_hat_prepared);
        let H = E::pairing(C2_prepared, issuer_pk.vk_prepared) - params.pairing;
        // e(C2, -g_hat) = e(-C2, g_hat)
        let J = E::pairing(
            E::G1Prepared::from(self.C.encrypted.into_group().neg()),
            params.g_hat_prepared,
        );
        // F , G, I are part of the precomputed public params
        let F = issuer_pk.w_g_hat;
        let G = issuer_pk.minus_g_w_hat;
        let I = issuer_pk.w_vk;
        let minus_challenge = challenge.neg();

        // Verify each response
        if self.t_C1 != (params.g * self.resp_beta + self.C.eph_pk * minus_challenge).into() {
            return Err(SyraError::InvalidProof);
        }
        if self.t_C1_hat
            != (params.g_hat * self.resp_alpha + self.C_hat.eph_pk * minus_challenge).into()
        {
            return Err(SyraError::InvalidProof);
        }
        if self.t_B != A * self.resp_alpha + B * minus_challenge {
            return Err(SyraError::InvalidProof);
        }
        if self.t_E != F * self.resp_beta + G * self.resp_alpha + E * minus_challenge {
            return Err(SyraError::InvalidProof);
        }
        let F_bs = F * self.resp_beta_times_s;
        if self.t_H != I * self.resp_beta + F_bs + J * self.resp_s + H * minus_challenge {
            return Err(SyraError::InvalidProof);
        }
        if self.t_K1 != F * self.resp_s + G * self.resp_r1 + self.K1 * minus_challenge {
            return Err(SyraError::InvalidProof);
        }
        let K2_c = self.K2 * minus_challenge;
        if self.t_K2 != F_bs + G * self.resp_r2 + K2_c {
            return Err(SyraError::InvalidProof);
        }
        if self.t_K2_product != E * self.resp_s + G * self.resp_r3 + K2_c {
            return Err(SyraError::InvalidProof);
        }
        Ok(())
    }

    pub fn challenge_contribution<W: Write>(
        &self,
        Z: &E::G1Affine,
        writer: W,
    ) -> Result<(), SyraError> {
        PseudonymGenProtocol::compute_challenge_contribution(
            Z,
            &self.T,
            &self.C,
            &self.C_hat,
            &self.K1,
            &self.K2,
            &self.t_C1,
            &self.t_C1_hat,
            &self.t_B,
            &self.t_E,
            &self.t_H,
            &self.t_K1,
            &self.t_K2,
            &self.t_K2_product,
            writer,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crate::setup::{IssuerPublicKey, IssuerSecretKey, SetupParams, UserSecretKey};
    use ark_bls12_381::{Bls12_381, Fr, G1Affine};
    use ark_std::rand::{rngs::StdRng, SeedableRng};
    use blake2::Blake2b512;
    use dock_crypto_utils::hashing_utils::affine_group_elem_from_try_and_incr;
    use schnorr_pok::compute_random_oracle_challenge;

    #[test]
    fn pseudonym() {
        let mut rng = StdRng::seed_from_u64(0u64);

        let params = SetupParams::<Bls12_381>::new::<Blake2b512>(b"test");

        // Signer's setup
        let isk = IssuerSecretKey::new(&mut rng);
        let ipk = IssuerPublicKey::new(&mut rng, &isk, &params);
        let prepared_ipk = PreparedIssuerPublicKey::new(ipk.clone(), params.clone());

        // Signer creates user secret key
        let user_id = compute_random_oracle_challenge::<Fr, Blake2b512>(b"low entropy user-id");
        let usk = UserSecretKey::new(user_id, &isk, params.clone());

        // Verifier gives message and context to user
        let context = b"test-context";
        let msg = b"test-message";

        // Generate Z from context
        let mut Z_bytes = vec![];
        Z_bytes.extend_from_slice(context);
        let Z = affine_group_elem_from_try_and_incr::<G1Affine, Blake2b512>(&Z_bytes);

        // User generates a pseudonym
        let start = Instant::now();
        let protocol = PseudonymGenProtocol::init(
            &mut rng,
            Z.clone(),
            user_id.clone(),
            None,
            &usk,
            prepared_ipk.clone(),
            params.clone(),
        );
        let mut chal_bytes = vec![];
        protocol
            .challenge_contribution(&Z, &mut chal_bytes)
            .unwrap();
        // Add message to the transcript (message contributes to challenge)
        chal_bytes.extend_from_slice(msg);
        let challenge_prover = compute_random_oracle_challenge::<Fr, Blake2b512>(&chal_bytes);
        let proof = protocol.gen_proof(&challenge_prover);
        println!("Time to create proof {:?}", start.elapsed());

        // Verifier checks the correctness of the pseudonym
        let start = Instant::now();
        let mut chal_bytes = vec![];
        proof.challenge_contribution(&Z, &mut chal_bytes).unwrap();
        // Add message to the transcript (message contributes to challenge)
        chal_bytes.extend_from_slice(msg);
        let challenge_verifier = compute_random_oracle_challenge::<Fr, Blake2b512>(&chal_bytes);
        proof
            .verify(&challenge_verifier, Z, prepared_ipk.clone(), params.clone())
            .unwrap();
        println!("Time to verify proof {:?}", start.elapsed());
    }
}
