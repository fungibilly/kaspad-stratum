// Written in 2014 by Andrew Poelstra <apoelstra@wpsoftware.net>
// SPDX-License-Identifier: CC0-1.0

//! Big unsigned integer types.
//!
//! Implementation of various large-but-fixed sized unsigned integer types.
//! The functions here are designed to be fast.

pub trait BitArray {
    /// Is bit set?
    fn bit(&self, idx: usize) -> bool;

    /// Returns an array which is just the bits from start to end
    fn bit_slice(&self, start: usize, end: usize) -> Self;

    /// Bitwise and with `n` ones
    fn mask(&self, n: usize) -> Self;

    /// Trailing zeros
    fn trailing_zeros(&self) -> usize;

    /// Create all-zeros value
    fn zero() -> Self;

    /// Create value representing one
    fn one() -> Self;
}

/// Implements standard array methods for a given wrapper type
macro_rules! impl_array_newtype {
    ($thing:ident, $ty:ty, $len:literal) => {
        impl $thing {
            /// Converts the object to a raw pointer
            #[inline]
            pub fn as_ptr(&self) -> *const $ty {
                let &$thing(ref dat) = self;
                dat.as_ptr()
            }

            /// Converts the object to a mutable raw pointer
            #[inline]
            pub fn as_mut_ptr(&mut self) -> *mut $ty {
                let &mut $thing(ref mut dat) = self;
                dat.as_mut_ptr()
            }

            /// Returns the length of the object as an array
            #[inline]
            pub fn len(&self) -> usize {
                $len
            }

            /// Returns whether the object, as an array, is empty. Always false.
            #[inline]
            pub fn is_empty(&self) -> bool {
                false
            }

            /// Returns the underlying bytes.
            #[inline]
            pub fn as_bytes(&self) -> &[$ty; $len] {
                &self.0
            }

            /// Returns the underlying bytes.
            #[inline]
            pub fn to_bytes(&self) -> [$ty; $len] {
                self.0.clone()
            }

            /// Returns the underlying bytes.
            #[inline]
            pub fn into_bytes(self) -> [$ty; $len] {
                self.0
            }
        }

        impl<'a> core::convert::From<&'a [$ty]> for $thing {
            fn from(data: &'a [$ty]) -> $thing {
                assert_eq!(data.len(), $len);
                let mut ret = [0; $len];
                ret.copy_from_slice(&data[..]);
                $thing(ret)
            }
        }

        impl<I> core::ops::Index<I> for $thing
        where
            [$ty]: core::ops::Index<I>,
        {
            type Output = <[$ty] as core::ops::Index<I>>::Output;

            #[inline]
            fn index(&self, index: I) -> &Self::Output {
                &self.0[index]
            }
        }
    };
}

macro_rules! construct_uint {
    ($name:ident, $n_words:literal) => {
        /// Little-endian large integer type
        #[derive(Copy, Clone, PartialEq, Eq, Hash, Default)]
        pub struct $name([u64; $n_words]);
        impl_array_newtype!($name, u64, $n_words);

        impl $name {
            #[inline]
            pub fn as_slice(&self) -> &[u64] {
                &self.0
            }

            /// Conversion to u32
            #[inline]
            pub fn low_u32(&self) -> u32 {
                let &$name(ref arr) = self;
                arr[0] as u32
            }

            /// Conversion to u64
            #[inline]
            pub fn low_u64(&self) -> u64 {
                let &$name(ref arr) = self;
                arr[0] as u64
            }

            /// Return the least number of bits needed to represent the number
            #[inline]
            pub fn bits(&self) -> usize {
                let &$name(ref arr) = self;
                for i in 1..$n_words {
                    if arr[$n_words - i] > 0 {
                        return (0x40 * ($n_words - i + 1))
                            - arr[$n_words - i].leading_zeros() as usize;
                    }
                }
                0x40 - arr[0].leading_zeros() as usize
            }

            /// Multiplication by u32
            pub fn mul_u32(self, other: u32) -> $name {
                let $name(ref arr) = self;
                let mut carry = [0u64; $n_words];
                let mut ret = [0u64; $n_words];
                for i in 0..$n_words {
                    let not_last_word = i < $n_words - 1;
                    let upper = other as u64 * (arr[i] >> 32);
                    let lower = other as u64 * (arr[i] & 0xFFFFFFFF);
                    if not_last_word {
                        carry[i + 1] += upper >> 32;
                    }
                    let (sum, overflow) = lower.overflowing_add(upper << 32);
                    ret[i] = sum;
                    if overflow && not_last_word {
                        carry[i + 1] += 1;
                    }
                }
                $name(ret) + $name(carry)
            }

            /// Create an object from a given unsigned 64-bit integer
            #[inline]
            pub fn from_u64(init: u64) -> Option<$name> {
                let mut ret = [0; $n_words];
                ret[0] = init;
                Some($name(ret))
            }

            /// Create an object from a given signed 64-bit integer
            #[inline]
            pub fn from_i64(init: i64) -> Option<$name> {
                if init >= 0 {
                    $name::from_u64(init as u64)
                } else {
                    None
                }
            }

            // divmod like operation, returns (quotient, remainder)
            #[inline]
            fn div_rem(self, other: Self) -> (Self, Self) {
                let mut sub_copy = self;
                let mut shift_copy = other;
                let mut ret = [0u64; $n_words];

                let my_bits = self.bits();
                let your_bits = other.bits();

                // Check for division by 0
                assert!(your_bits != 0, "attempted to divide by zero");

                // Early return in case we are dividing by a larger number than us
                if my_bits < your_bits {
                    return ($name(ret), sub_copy);
                }

                // Bitwise long division
                let mut shift = my_bits - your_bits;
                shift_copy = shift_copy << shift;
                loop {
                    if sub_copy >= shift_copy {
                        ret[shift / 64] |= 1 << (shift % 64);
                        sub_copy = sub_copy - shift_copy;
                    }
                    shift_copy = shift_copy >> 1;
                    if shift == 0 {
                        break;
                    }
                    shift -= 1;
                }

                ($name(ret), sub_copy)
            }

            /// Increment by 1
            #[inline]
            pub fn increment(&mut self) {
                let &mut $name(ref mut arr) = self;
                for i in 0..$n_words {
                    arr[i] = arr[i].wrapping_add(1);
                    if arr[i] != 0 {
                        break;
                    }
                }
            }
        }

        impl From<[u64; $n_words]> for $name {
            fn from(v: [u64; $n_words]) -> Self {
                $name(v)
            }
        }

        impl PartialOrd for $name {
            #[inline]
            fn partial_cmp(&self, other: &$name) -> Option<core::cmp::Ordering> {
                Some(self.cmp(&other))
            }
        }

        impl Ord for $name {
            #[inline]
            fn cmp(&self, other: &$name) -> core::cmp::Ordering {
                // We need to manually implement ordering because we use little-endian
                // and the auto derive is a lexicographic ordering(i.e. memcmp)
                // which with numbers is equivalent to big-endian
                for i in 0..$n_words {
                    if self[$n_words - 1 - i] < other[$n_words - 1 - i] {
                        return core::cmp::Ordering::Less;
                    }
                    if self[$n_words - 1 - i] > other[$n_words - 1 - i] {
                        return core::cmp::Ordering::Greater;
                    }
                }
                core::cmp::Ordering::Equal
            }
        }

        impl core::ops::Add<$name> for $name {
            type Output = $name;

            fn add(self, other: $name) -> $name {
                let $name(ref me) = self;
                let $name(ref you) = other;
                let mut ret = [0u64; $n_words];
                let mut carry = [0u64; $n_words];
                let mut b_carry = false;
                for i in 0..$n_words {
                    ret[i] = me[i].wrapping_add(you[i]);
                    if i < $n_words - 1 && ret[i] < me[i] {
                        carry[i + 1] = 1;
                        b_carry = true;
                    }
                }
                if b_carry {
                    $name(ret) + $name(carry)
                } else {
                    $name(ret)
                }
            }
        }

        impl core::ops::Sub<$name> for $name {
            type Output = $name;

            #[inline]
            fn sub(self, other: $name) -> $name {
                self + !other + $crate::uint::BitArray::one()
            }
        }

        impl core::ops::Mul<$name> for $name {
            type Output = $name;

            fn mul(self, other: $name) -> $name {
                use $crate::uint::BitArray;
                let mut me = $name::zero();
                // TODO: be more efficient about this
                for i in 0..(2 * $n_words) {
                    let to_mul = (other >> (32 * i)).low_u32();
                    me = me + (self.mul_u32(to_mul) << (32 * i));
                }
                me
            }
        }

        impl core::ops::Div<$name> for $name {
            type Output = $name;

            fn div(self, other: $name) -> $name {
                self.div_rem(other).0
            }
        }

        impl core::ops::Rem<$name> for $name {
            type Output = $name;

            fn rem(self, other: $name) -> $name {
                self.div_rem(other).1
            }
        }

        impl $crate::uint::BitArray for $name {
            #[inline]
            fn bit(&self, index: usize) -> bool {
                let &$name(ref arr) = self;
                arr[index / 64] & (1 << (index % 64)) != 0
            }

            #[inline]
            fn bit_slice(&self, start: usize, end: usize) -> $name {
                (*self >> start).mask(end - start)
            }

            #[inline]
            fn mask(&self, n: usize) -> $name {
                let &$name(ref arr) = self;
                let mut ret = [0; $n_words];
                for i in 0..$n_words {
                    if n >= 0x40 * (i + 1) {
                        ret[i] = arr[i];
                    } else {
                        ret[i] = arr[i] & ((1 << (n - 0x40 * i)) - 1);
                        break;
                    }
                }
                $name(ret)
            }

            #[inline]
            fn trailing_zeros(&self) -> usize {
                let &$name(ref arr) = self;
                for i in 0..($n_words - 1) {
                    if arr[i] > 0 {
                        return (0x40 * i) + arr[i].trailing_zeros() as usize;
                    }
                }
                (0x40 * ($n_words - 1)) + arr[$n_words - 1].trailing_zeros() as usize
            }

            fn zero() -> $name {
                Default::default()
            }
            fn one() -> $name {
                $name({
                    let mut ret = [0; $n_words];
                    ret[0] = 1;
                    ret
                })
            }
        }

        impl core::ops::BitAnd<$name> for $name {
            type Output = $name;

            #[inline]
            fn bitand(self, other: $name) -> $name {
                let $name(ref arr1) = self;
                let $name(ref arr2) = other;
                let mut ret = [0u64; $n_words];
                for i in 0..$n_words {
                    ret[i] = arr1[i] & arr2[i];
                }
                $name(ret)
            }
        }

        impl core::ops::BitXor<$name> for $name {
            type Output = $name;

            #[inline]
            fn bitxor(self, other: $name) -> $name {
                let $name(ref arr1) = self;
                let $name(ref arr2) = other;
                let mut ret = [0u64; $n_words];
                for i in 0..$n_words {
                    ret[i] = arr1[i] ^ arr2[i];
                }
                $name(ret)
            }
        }

        impl core::ops::BitOr<$name> for $name {
            type Output = $name;

            #[inline]
            fn bitor(self, other: $name) -> $name {
                let $name(ref arr1) = self;
                let $name(ref arr2) = other;
                let mut ret = [0u64; $n_words];
                for i in 0..$n_words {
                    ret[i] = arr1[i] | arr2[i];
                }
                $name(ret)
            }
        }

        impl core::ops::Not for $name {
            type Output = $name;

            #[inline]
            fn not(self) -> $name {
                let $name(ref arr) = self;
                let mut ret = [0u64; $n_words];
                for i in 0..$n_words {
                    ret[i] = !arr[i];
                }
                $name(ret)
            }
        }

        impl core::ops::Shl<usize> for $name {
            type Output = $name;

            fn shl(self, shift: usize) -> $name {
                let $name(ref original) = self;
                let mut ret = [0u64; $n_words];
                let word_shift = shift / 64;
                let bit_shift = shift % 64;
                for i in 0..$n_words {
                    // Shift
                    if bit_shift < 64 && i + word_shift < $n_words {
                        ret[i + word_shift] += original[i] << bit_shift;
                    }
                    // Carry
                    if bit_shift > 0 && i + word_shift + 1 < $n_words {
                        ret[i + word_shift + 1] += original[i] >> (64 - bit_shift);
                    }
                }
                $name(ret)
            }
        }

        impl core::ops::Shr<usize> for $name {
            type Output = $name;

            fn shr(self, shift: usize) -> $name {
                let $name(ref original) = self;
                let mut ret = [0u64; $n_words];
                let word_shift = shift / 64;
                let bit_shift = shift % 64;
                for i in word_shift..$n_words {
                    // Shift
                    ret[i - word_shift] += original[i] >> bit_shift;
                    // Carry
                    if bit_shift > 0 && i < $n_words - 1 {
                        ret[i - word_shift] += original[i + 1] << (64 - bit_shift);
                    }
                }
                $name(ret)
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                let &$name(ref data) = self;
                write!(f, "0x")?;
                for ch in data.iter().rev() {
                    write!(f, "{:016x}", ch)?;
                }
                Ok(())
            }
        }
    };
}

construct_uint!(U256, 4);
