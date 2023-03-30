// Copyright © Aptos Foundation

// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "testing")]
use crate::natives::cryptography::ristretto255::pop_scalar_from_bytes;
use crate::natives::cryptography::ristretto255_point::{
    get_point_handle, NativeRistrettoPointContext,
};
#[cfg(feature = "testing")]
use crate::natives::helpers::make_test_only_safe_native;
use aptos_crypto::bulletproofs::MAX_RANGE_BITS;
use bulletproofs::{BulletproofGens, PedersenGens};
#[cfg(feature = "testing")]
use byteorder::{ByteOrder, LittleEndian};
use curve25519_dalek::ristretto::CompressedRistretto;
use merlin::Transcript;
use move_core_types::gas_algebra::{
    InternalGas, InternalGasPerArg, InternalGasPerByte, NumArgs, NumBytes,
};
use move_vm_runtime::native_functions::{NativeFunction};
use move_vm_types::loaded_data::runtime_types::Type;
use move_vm_types::values::{StructRef, Value};
use once_cell::sync::Lazy;
use smallvec::{smallvec, SmallVec};
use std::collections::VecDeque;
use std::sync::Arc;
use aptos_types::on_chain_config::{Features, TimedFeatures};
use crate::natives::helpers::{make_safe_native, SafeNativeContext, SafeNativeError, SafeNativeResult};
use crate::safely_pop_arg;

pub mod abort_codes {
    /// Abort code when deserialization fails (leading 0x01 == INVALID_ARGUMENT)
    /// NOTE: This must match the code in the Move implementation
    pub const NFE_DESERIALIZE_RANGE_PROOF: u64 = 0x01_0001;

    /// Abort code when input value for a range proof is too large.
    /// NOTE: This must match the code in the Move implementation
    pub const NFE_VALUE_OUTSIDE_RANGE: u64 = 0x01_0002;

    /// Abort code when the request range is too large than the maximum supported one.
    /// NOTE: This must match the code in the Move implementation
    pub const NFE_RANGE_NOT_SUPPORTED: u64 = 0x01_0003;
}

/// The Bulletproofs library only seems to support proving [0, 2^{num_bits}) ranges where num_bits is
/// either 8, 16, 32 or 64.
fn is_supported_number_of_bits(num_bits: usize) -> bool {
    matches!(num_bits, 8 | 16 | 32 | 64)
}

/// Default Pedersen commitment key compatible with the default Bulletproof verification API.
static PEDERSEN_GENERATORS: Lazy<PedersenGens> = Lazy::new(PedersenGens::default);

/// Public parameters of the Bulletproof range proof system
static BULLETPROOF_GENERATORS: Lazy<BulletproofGens> =
    Lazy::new(|| BulletproofGens::new(MAX_RANGE_BITS, 1));

fn native_verify_range_proof_custom_ck(
    gas_params: &GasParameters,
    context: &mut SafeNativeContext,
    _ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> SafeNativeResult<SmallVec<[Value; 1]>> {
    debug_assert!(_ty_args.is_empty());
    debug_assert!(args.len() == 6);

    let dst = safely_pop_arg!(args, Vec<u8>);
    let num_bits = safely_pop_arg!(args, u64) as usize;
    let proof_bytes = safely_pop_arg!(args, Vec<u8>);
    let rand_base_handle = get_point_handle(&safely_pop_arg!(args, StructRef))?;
    let val_base_handle = get_point_handle(&safely_pop_arg!(args, StructRef))?;
    let comm_bytes = safely_pop_arg!(args, Vec<u8>);

    let comm_point = CompressedRistretto::from_slice(comm_bytes.as_slice());

    if !is_supported_number_of_bits(num_bits) {
        return Err(SafeNativeError::Abort {
            abort_code: abort_codes::NFE_RANGE_NOT_SUPPORTED,
        });
    }

    let pg = {
        let point_context = context.extensions().get::<NativeRistrettoPointContext>();
        let point_data = point_context.point_data.borrow_mut();

        let rand_base = point_data.get_point(&rand_base_handle);
        let val_base = point_data.get_point(&val_base_handle);

        // TODO(Perf): Is there a way to avoid this unnecessary cloning here?
        PedersenGens {
            B: *val_base,
            B_blinding: *rand_base,
        }
    };

    gas_params.verify_range_proof(context, &comm_point, &pg, &proof_bytes[..], num_bits, dst)
}

fn native_verify_range_proof(
    gas_params: &GasParameters,
    context: &mut SafeNativeContext,
    _ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> SafeNativeResult<SmallVec<[Value; 1]>> {
    debug_assert!(_ty_args.is_empty());
    debug_assert!(args.len() == 4);

    let dst = safely_pop_arg!(args, Vec<u8>);
    let num_bits = safely_pop_arg!(args, u64) as usize;

    if !is_supported_number_of_bits(num_bits) {
        return Err(SafeNativeError::Abort {
            abort_code: abort_codes::NFE_RANGE_NOT_SUPPORTED,
        });
    }

    let proof_bytes = safely_pop_arg!(args, Vec<u8>);
    let comm_bytes = safely_pop_arg!(args, Vec<u8>);
    let comm_point = CompressedRistretto::from_slice(comm_bytes.as_slice());

    gas_params.verify_range_proof(
        context,
        &comm_point,
        &PEDERSEN_GENERATORS,
        &proof_bytes[..],
        num_bits,
        dst,
    )
}

#[cfg(feature = "testing")]
/// This is a test-only native that charges zero gas. It is only exported in testing mode.
fn native_test_only_prove_range(
    _context: &mut SafeNativeContext,
    _ty_args: Vec<Type>,
    mut args: VecDeque<Value>,
) -> SafeNativeResult<SmallVec<[Value; 1]>> {
    debug_assert!(_ty_args.is_empty());
    debug_assert!(args.len() == 4);

    let dst = safely_pop_arg!(args, Vec<u8>);
    let num_bits = safely_pop_arg!(args, u64) as usize;
    let v_blinding = pop_scalar_from_bytes(&mut args)?;
    let v = pop_scalar_from_bytes(&mut args)?;

    if !is_supported_number_of_bits(num_bits) {
        return Err(SafeNativeError::Abort {
            abort_code: abort_codes::NFE_RANGE_NOT_SUPPORTED,
        });
    }

    // Make sure only the first 64 bits are set.
    if !v.as_bytes()[8..].iter().all(|&byte| byte == 0u8) {
        return Err(SafeNativeError::Abort {
            abort_code: abort_codes::NFE_VALUE_OUTSIDE_RANGE,
        });
    }

    // Convert Scalar to u64.
    let v = LittleEndian::read_u64(v.as_bytes());

    let mut t = Transcript::new(dst.as_slice());

    // Construct a range proof.
    let (proof, commitment) = bulletproofs::RangeProof::prove_single(
        &BULLETPROOF_GENERATORS,
        &PEDERSEN_GENERATORS,
        &mut t,
        v,
        &v_blinding,
        num_bits,
    )
    .expect("Bulletproofs prover failed unexpectedly");

    Ok(smallvec![Value::vector_u8(proof.to_bytes()),
            Value::vector_u8(commitment.as_bytes().to_vec())])
}

/***************************************************************************************************
 * module
 *
 **************************************************************************************************/
#[derive(Debug, Clone)]
pub struct GasParameters {
    pub base: InternalGas,
    pub per_byte_rangeproof_deserialize: InternalGasPerByte,
    pub per_bit_rangeproof_verify: InternalGasPerArg,
}

impl GasParameters {
    /// Helper function to gas meter and verify a single Bulletproof range proof for a Pedersen
    /// commitment with `pc_gens` as its commitment key.
    fn verify_range_proof(
        &self,
        context: &mut SafeNativeContext,
        comm_point: &CompressedRistretto,
        pc_gens: &PedersenGens,
        proof_bytes: &[u8],
        bit_length: usize,
        dst: Vec<u8>,
    ) -> SafeNativeResult<SmallVec<[Value; 1]>> {
        context.charge(self.base)?;
        context.charge(self.per_byte_rangeproof_deserialize * NumBytes::new(proof_bytes.len() as u64))?;

        let range_proof = match bulletproofs::RangeProof::from_bytes(proof_bytes) {
            Ok(proof) => proof,
            Err(_) => {
                return Err(SafeNativeError::Abort {
                    abort_code: abort_codes::NFE_DESERIALIZE_RANGE_PROOF,
                })
            }
        };

        // The (Bullet)proof size is $\log_2(num_bits)$ and its verification time is $O(num_bits)$
        // TODO: But an MSM is used so we should account for that
        context.charge(self.per_bit_rangeproof_verify * NumArgs::new(bit_length as u64))?;

        let mut ver_trans = Transcript::new(dst.as_slice());

        let success = range_proof
            .verify_single(
                &BULLETPROOF_GENERATORS,
                pc_gens,
                &mut ver_trans,
                comm_point,
                bit_length,
            )
            .is_ok();

        Ok(smallvec![Value::bool(success)])
    }
}

pub fn make_all(gas_params: GasParameters, timed_features: TimedFeatures, features: Arc<Features>) -> impl Iterator<Item = (String, NativeFunction)> {
    let mut natives = vec![];

    #[cfg(feature = "testing")]
    natives.append(&mut vec![(
        "prove_range_internal",
        make_test_only_safe_native(timed_features.clone(), features.clone(), native_test_only_prove_range),
    )]);

    natives.append(&mut vec![
        (
            "verify_range_proof_custom_ck_internal",
            make_safe_native(gas_params.clone(), timed_features.clone(), features.clone(), native_verify_range_proof_custom_ck),
        ),
        (
            "verify_range_proof_internal",
            make_safe_native(gas_params, timed_features, features, native_verify_range_proof),
        ),
    ]);

    crate::natives::helpers::make_module_natives(natives)
}
