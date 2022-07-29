use crate::uint::{BitArray, U256};
use std::ops::Not;

pub fn u256_from_compact_target(bits: u32) -> U256 {
    let (mant, expt) = {
        let unshifted_expt = bits >> 24;
        if unshifted_expt <= 3 {
            ((bits & 0xFFFFFF) >> (8 * (3 - unshifted_expt as usize)), 0)
        } else {
            (bits & 0xFFFFFF, 8 * ((bits >> 24) - 3))
        }
    };

    // The mantissa is signed but may not be negative
    if mant > 0x7FFFFF {
        Default::default()
    } else {
        U256::from_u64(mant as u64).unwrap() << (expt as usize)
    }
}

pub fn difficulty(mut target: U256) -> u64 {
    target.increment();
    ((U256::one() << 255) / target).low_u64()
}
