# TBD

[![CI](https://github.com/docknetwork/crypto/actions/workflows/test.yml/badge.svg)](https://github.com/docknetwork/crypto/actions/workflows/test.yml)
[![Apache-2](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://github.com/docknetwork/crypto/blob/main/LICENSE)
[![Dependencies](https://deps.rs/repo/github/docknetwork/crypto/status.svg)](https://deps.rs/repo/github/docknetwork/crypto)

Library providing privacy enhancing cryptographic primitives.

## Primitives

1. [Schnorr proof of knowledge protocol](./schnorr_pok) to prove knowledge of discrete log. [This](https://crypto.stanford.edu/cs355/19sp/lec5.pdf) is a good reference. 
2. [BBS+ signature](./bbs_plus) for anonymous credentials. Based on the paper [Anonymous Attestation Using the Strong Diffie Hellman Assumption Revisited](https://eprint.iacr.org/2016/663)
3. [Dynamic accumulators, both positive and universal](./vb_accumulator). Based on the paper [Dynamic Universal Accumulator with Batch Update over Bilinear Groups](https://eprint.iacr.org/2020/777)
4. [Composite proof system](./proof_system) that combines above primitives for use cases like 
   - prove knowledge of a BBS+ signature and the corresponding messages
   - prove knowledge of a modified PS signature and the corresponding messages
   - equality of signed messages (from same or different signatures) in zero knowledge
   - the (non)membership of a certain signed message(s)in the accumulator
   - numeric bounds (min, max) on the messages can be proved in zero-knowledge 
   - verifiable encryption of signed messages under BBS+. 
   - zk-SNARK created from R1CS and WASM generated by [Circom](https://docs.circom.io/) with witnesses as BBS+ signed messages (not exclusively though). 
5. [Verifiable encryption](./saver) using [SAVER](https://eprint.iacr.org/2019/1270).
6. [Compression and amortization of Sigma protocols](./compressed_sigma). This is PoC implementation.
7. [Secret sharing schemes and DKG](./secret_sharing_and_dkg). Implements verifiable secret sharing schemes and DKG from Gennaro and FROST.
8. [Cocount and PS signatures](./coconut/). Based on the paper [Security Analysis of Coconut, an Attribute-Based Credential Scheme with Threshold Issuance](https://eprint.iacr.org/2022/011)
9. [LegoGroth16](./legogroth16/).  LegoGroth16, the [LegoSNARK](https://eprint.iacr.org/2019/142) variant of [Groth16](https://eprint.iacr.org/2016/260) zkSNARK proof system

## Composite proof system

The [proof system](./proof_system) that uses above-mentioned primitives. 

## Build

`cargo build` or `cargo build --release`

By default, it uses standard library and [rayon](https://github.com/rayon-rs/rayon) for parallelization

To build with standard library but without parallelization, use `cargo build --no-default-features --features=std`

For `no_std` support, build as `cargo build --no-default-features --features=wasmer-sys`

For WASM, build as `cargo build --no-default-features --features=wasmer-js --target wasm32-unknown-unknown`

## Test

`cargo test`

The above maybe slower as it runs the tests in debug mode and some tests work on large inputs. 
For running tests faster, run `cargo test --release`


## Benchmarking

[Criterion](https://github.com/bheisler/criterion.rs) benchmarks [here](./benches)

Some tests also print time consumed by the operations, run `cargo test --release -- --nocapure [test name]`

## WASM wrapper

A WASM wrapper has been created over this repo [here](https://github.com/docknetwork/crypto-wasm). The wrapper is then used to create [this Typescript library](https://github.com/docknetwork/crypto-wasm-ts) which is more ergonomic than using the wrapper as the wrapper contains free floating functions.
