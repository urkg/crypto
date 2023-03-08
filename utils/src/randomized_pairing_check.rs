use ark_ec::pairing::{MillerLoopOutput, PairingOutput};
use ark_ec::{pairing::Pairing, AffineRepr, Group};
use ark_ff::{One, PrimeField, Zero};
use ark_std::rand::Rng;
use ark_std::{cfg_iter, ops::MulAssign, vec, vec::Vec, UniformRand};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Inspired from Snarkpack implementation - https://github.com/nikkolasg/snarkpack/blob/main/src/pairing_check.rs
/// RandomizedPairingChecker represents a check of the form e(A,B)e(C,D)... = T. Checks can
/// be aggregated together using random linear combination. The efficiency comes
/// from keeping the results from the miller loop output before proceeding to a final
/// exponentiation when verifying if all checks are verified.
/// For each pairing equation, multiply by a power of a random element created during initialization
#[derive(Debug, Clone)]
pub struct RandomizedPairingChecker<E: Pairing> {
    /// a miller loop result that is to be multiplied by other miller loop results
    /// before going into a final exponentiation result
    left: MillerLoopOutput<E>,
    /// a right side result which is already in the right subgroup Gt which is to
    /// be compared to the left side when "final_exponentiatiat"-ed
    right: PairingOutput<E>,
    /// If true, delays the computation of miller loops till the end (unless overridden) trading off memory for CPU time.
    lazy: bool,
    /// Keeps the pairs of G1, G2 elements that need to be used in miller loops when running lazily
    pending: (Vec<E::G1Prepared>, Vec<E::G2Prepared>),
    random: E::ScalarField,
    /// For each pairing equation, its multiplied by `self.random`
    current_random: E::ScalarField,
}

impl<E> RandomizedPairingChecker<E>
where
    E: Pairing,
{
    pub fn new_using_rng<R: Rng>(rng: &mut R, lazy: bool) -> Self {
        Self::new(E::ScalarField::rand(rng), lazy)
    }

    pub fn new(random: E::ScalarField, lazy: bool) -> Self {
        Self {
            left: MillerLoopOutput(E::TargetField::one()),
            right: PairingOutput::zero(),
            lazy,
            pending: (vec![], vec![]),
            random,
            current_random: E::ScalarField::one(),
        }
    }

    pub fn add_multiple_sources_and_target(
        &mut self,
        a: &[E::G1Affine],
        b: impl IntoIterator<Item = impl Into<E::G2Prepared>>,
        out: &PairingOutput<E>,
    ) {
        self.add_multiple_sources_and_target_with_laziness_choice(a, b, out, self.lazy)
    }

    pub fn add_multiple_sources(
        &mut self,
        a: &[E::G1Affine],
        b: impl IntoIterator<Item = impl Into<E::G2Prepared>>,
        c: &[E::G1Affine],
        d: impl IntoIterator<Item = impl Into<E::G2Prepared>>,
    ) {
        self.add_multiple_sources_with_laziness_choice(a, b, c, d, self.lazy)
    }

    pub fn add_sources(
        &mut self,
        a: &E::G1Affine,
        b: impl Into<E::G2Prepared>,
        c: &E::G1Affine,
        d: impl Into<E::G2Prepared>,
    ) {
        self.add_sources_with_laziness_choice(a, b, c, d, self.lazy)
    }

    pub fn add_multiple_sources_and_target_with_laziness_choice(
        &mut self,
        a: &[E::G1Affine],
        b: impl IntoIterator<Item = impl Into<E::G2Prepared>>,
        out: &PairingOutput<E>,
        lazy: bool,
    ) {
        let m = self.current_random.into_bigint();
        let mut a_m = cfg_iter!(a)
            .map(|a| E::G1Prepared::from(a.mul_bigint(m)))
            .collect::<Vec<_>>();
        if lazy {
            self.pending.0.append(&mut a_m);
            self.pending
                .1
                .append(&mut b.into_iter().map(|b| b.into()).collect());
        } else {
            self.left.0.mul_assign(E::multi_miller_loop(a_m, b).0);
        }
        self.right += out.mul_bigint(m);
        self.current_random *= self.random;
    }

    pub fn add_multiple_sources_with_laziness_choice(
        &mut self,
        a: &[E::G1Affine],
        b: impl IntoIterator<Item = impl Into<E::G2Prepared>>,
        c: &[E::G1Affine],
        d: impl IntoIterator<Item = impl Into<E::G2Prepared>>,
        lazy: bool,
    ) {
        let m = self.current_random.into_bigint();
        let mut a_m = cfg_iter!(a)
            .map(|a| E::G1Prepared::from(a.mul_bigint(m)))
            .collect::<Vec<_>>();
        let mut c_m = cfg_iter!(c)
            .map(|c| E::G1Prepared::from(-c.mul_bigint(m)))
            .collect::<Vec<_>>();
        if lazy {
            self.pending.0.append(&mut a_m);
            self.pending
                .1
                .append(&mut b.into_iter().map(|b| b.into()).collect());
            self.pending.0.append(&mut c_m);
            self.pending
                .1
                .append(&mut d.into_iter().map(|d| d.into()).collect());
        } else {
            self.left.0.mul_assign(E::multi_miller_loop(a_m, b).0);
            self.left.0.mul_assign(E::multi_miller_loop(c_m, d).0);
        }
        self.current_random *= self.random;
    }

    pub fn add_sources_with_laziness_choice(
        &mut self,
        a: &E::G1Affine,
        b: impl Into<E::G2Prepared>,
        c: &E::G1Affine,
        d: impl Into<E::G2Prepared>,
        lazy: bool,
    ) {
        let m = self.current_random.into_bigint();
        let mut g1 = vec![
            E::G1Prepared::from(a.mul_bigint(m)),
            E::G1Prepared::from(-c.mul_bigint(m)),
        ];
        let mut g2 = vec![b.into(), d.into()];
        if lazy {
            self.pending.0.append(&mut g1);
            self.pending.1.append(&mut g2);
        } else {
            self.left.0.mul_assign(E::multi_miller_loop(g1, g2).0);
        }
        self.current_random *= self.random;
    }

    pub fn verify(&self) -> bool {
        assert_eq!(self.pending.0.len(), self.pending.1.len());
        let left = if self.pending.0.len() > 0 {
            let mut p = E::multi_miller_loop(self.pending.0.clone(), self.pending.1.clone());
            p.0.mul_assign(self.left.0);
            p
        } else {
            self.left
        };
        E::final_exponentiation(left).unwrap() == self.right
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ark_bls12_381::{Bls12_381, G1Projective, G2Projective};
    use ark_ec::bls12::G2Prepared;
    use ark_ec::CurveGroup;
    use ark_std::rand::prelude::StdRng;
    use ark_std::rand::SeedableRng;
    use ark_std::UniformRand;
    use std::time::Instant;

    fn rev_vec<T: Clone>(v: &[T]) -> Vec<T> {
        let mut x = v.to_vec();
        x.reverse();
        x
    }

    #[test]
    fn test_pairing_randomize() {
        let mut rng = StdRng::seed_from_u64(0u64);
        let n = 10;
        let mut t1 = 0;

        let a1 = (0..n)
            .map(|_| G1Projective::rand(&mut rng).into_affine())
            .collect::<Vec<_>>();
        let b1 = (0..n)
            .map(|_| G2Projective::rand(&mut rng).into_affine())
            .collect::<Vec<_>>();
        let a2 = (0..n + 5)
            .map(|_| G1Projective::rand(&mut rng).into_affine())
            .collect::<Vec<_>>();
        let b2 = (0..n + 5)
            .map(|_| G2Projective::rand(&mut rng).into_affine())
            .collect::<Vec<_>>();
        let a3 = (0..n - 2)
            .map(|_| G1Projective::rand(&mut rng).into_affine())
            .collect::<Vec<_>>();
        let b3 = (0..n - 2)
            .map(|_| G2Projective::rand(&mut rng).into_affine())
            .collect::<Vec<_>>();

        let start = Instant::now();
        let out1 = Bls12_381::multi_pairing(a1.clone(), b1.clone());
        t1 += start.elapsed().as_micros();

        let start = Instant::now();
        let out2 = Bls12_381::multi_pairing(a2.clone(), b2.clone());
        t1 += start.elapsed().as_micros();

        let start = Instant::now();
        let out3 = Bls12_381::multi_pairing(a3.clone(), b3.clone());
        t1 += start.elapsed().as_micros();

        println!("Time taken without checker {} us", t1);

        for lazy in [true, false] {
            let start = Instant::now();
            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_multiple_sources_and_target(&a1, &b1, &out1);
            checker.add_multiple_sources_and_target(&a2, &b2, &out2);
            checker.add_multiple_sources_and_target(&a3, &b3, &out3);
            assert!(checker.verify());
            let l_str = if lazy { "lazy-" } else { "" };
            println!(
                "Time taken with {}checker {} us",
                l_str,
                start.elapsed().as_micros()
            );
        }

        let b1_prep = b1.iter().map(|b| G2Prepared::from(*b)).collect::<Vec<_>>();
        let b2_prep = b2.iter().map(|b| G2Prepared::from(*b)).collect::<Vec<_>>();
        let b3_prep = b3.iter().map(|b| G2Prepared::from(*b)).collect::<Vec<_>>();

        for lazy in [true, false] {
            let start = Instant::now();
            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_multiple_sources_and_target(&a1, b1_prep.clone(), &out1);
            checker.add_multiple_sources_and_target(&a2, b2_prep.clone(), &out2);
            checker.add_multiple_sources_and_target(&a3, b3_prep.clone(), &out3);
            assert!(checker.verify());
            let l_str = if lazy { "lazy-" } else { "" };
            println!(
                "Time taken with prepared G2 and {}checker {} us",
                l_str,
                start.elapsed().as_micros()
            );
        }

        let a1_rev = rev_vec(&a1);
        let a2_rev = rev_vec(&a2);
        let a3_rev = rev_vec(&a3);
        let b1_rev = rev_vec(&b1);
        let b2_rev = rev_vec(&b2);
        let b3_rev = rev_vec(&b3);

        let b1_rev_prep = b1_rev
            .iter()
            .map(|b| G2Prepared::from(*b))
            .collect::<Vec<_>>();
        let b2_rev_prep = b2_rev
            .iter()
            .map(|b| G2Prepared::from(*b))
            .collect::<Vec<_>>();
        let b3_rev_prep = b3_rev
            .iter()
            .map(|b| G2Prepared::from(*b))
            .collect::<Vec<_>>();

        for lazy in [true, false] {
            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_multiple_sources(&a1, &b1, &a1_rev, &b1_rev);
            assert!(checker.verify());
        }

        for lazy in [true, false] {
            let start = Instant::now();
            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_multiple_sources(&a1, &b1, &a1_rev, &b1_rev);
            checker.add_multiple_sources(&a2, &b2, &a2_rev, &b2_rev);
            checker.add_multiple_sources(&a3, &b3, &a3_rev, &b3_rev);
            assert!(checker.verify());
            let l_str = if lazy { "lazy-" } else { "" };
            println!(
                "Time taken with {}checker {} us",
                l_str,
                start.elapsed().as_micros()
            );
        }

        for lazy in [true, false] {
            let start = Instant::now();
            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_multiple_sources(&a1, b1_prep.clone(), &a1_rev, b1_rev_prep.clone());
            checker.add_multiple_sources(&a2, b2_prep.clone(), &a2_rev, b2_rev_prep.clone());
            checker.add_multiple_sources(&a3, b3_prep.clone(), &a3_rev, b3_rev_prep.clone());
            assert!(checker.verify());
            let l_str = if lazy { "lazy-" } else { "" };
            println!(
                "Time taken with prepared G2 and {}checker {} us",
                l_str,
                start.elapsed().as_micros()
            );
        }

        for lazy in [true, false] {
            let start = Instant::now();
            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_multiple_sources_and_target(&a1, &b1, &out1);
            checker.add_multiple_sources_and_target(&a2, &b2, &out2);
            checker.add_multiple_sources_and_target(&a3, &b3, &out3);
            checker.add_multiple_sources(&a1, &b1, &a1_rev, &b1_rev);
            checker.add_multiple_sources(&a2, &b2, &a2_rev, &b2_rev);
            checker.add_multiple_sources(&a3, &b3, &a3_rev, &b3_rev);
            assert!(checker.verify());
            let l_str = if lazy { "lazy-" } else { "" };
            println!(
                "Time taken with {}checker {} us",
                l_str,
                start.elapsed().as_micros()
            );
        }

        for lazy in [true, false] {
            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_sources(&a1[0], b1[0].clone(), &a1[0], b1[0].clone());
            assert!(checker.verify());

            let mut checker = RandomizedPairingChecker::<Bls12_381>::new_using_rng(&mut rng, lazy);
            checker.add_sources(&a1[0], b1[0].clone(), &a1[0], b1[0].clone());
            checker.add_sources(&a1[1], b1[1].clone(), &a1[1], b1[1].clone());
            checker.add_sources(&a1[2], b1[2].clone(), &a1[2], b1[2].clone());
            assert!(checker.verify());
        }
    }
}
