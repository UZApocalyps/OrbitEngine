//! Ed25519 (RFC 8032), ported from TweetNaCl's `crypto_sign` (public domain,
//! <https://tweetnacl.cr.yp.to>): field elements are 16 signed 16-bit limbs
//! in `i64`, points use extended coordinates. Added: the `S < L` check on
//! verification (RFC 8032 section 5.1.7), and the API works on detached
//! signatures.

#![allow(clippy::needless_range_loop)] // index-heavy port of the C code

use crate::sha512;

pub const SEED_LEN: usize = 32;
pub const PUBLIC_KEY_LEN: usize = 32;
pub const SIGNATURE_LEN: usize = 64;

type Gf = [i64; 16];

const GF0: Gf = [0; 16];
const GF1: Gf = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const D: Gf = [
    0x78a3, 0x1359, 0x4dca, 0x75eb, 0xd8ab, 0x4141, 0x0a4d, 0x0070, 0xe898, 0x7779, 0x4079, 0x8cc7,
    0xfe73, 0x2b6f, 0x6cee, 0x5203,
];
const D2: Gf = [
    0xf159, 0x26b2, 0x9b94, 0xebd6, 0xb156, 0x8283, 0x149a, 0x00e0, 0xd130, 0xeef3, 0x80f2, 0x198e,
    0xfce7, 0x56df, 0xd9dc, 0x2406,
];
const X: Gf = [
    0xd51a, 0x8f25, 0x2d60, 0xc956, 0xa7b2, 0x9525, 0xc760, 0x692c, 0xdc5c, 0xfdd6, 0xe231, 0xc0a4,
    0x53fe, 0xcd6e, 0x36d3, 0x2169,
];
const Y: Gf = [
    0x6658, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666,
    0x6666, 0x6666, 0x6666, 0x6666,
];
const I: Gf = [
    0xa0b0, 0x4a0e, 0x1b27, 0xc4ee, 0xe478, 0xad2f, 0x1806, 0x2f43, 0xd7a7, 0x3dfb, 0x0099, 0x2b4d,
    0xdf0b, 0x4fc1, 0x2480, 0x2b83,
];
/// The group order, little endian.
const L: [i64; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

fn car25519(o: &mut Gf) {
    for i in 0..16 {
        o[i] += 1 << 16;
        let c = o[i] >> 16;
        if i < 15 {
            o[i + 1] += c - 1;
        } else {
            o[0] += 38 * (c - 1);
        }
        o[i] -= c << 16;
    }
}

/// Swaps `p` and `q` when `b` is 1 (constant time).
fn sel25519(p: &mut Gf, q: &mut Gf, b: i64) {
    let c = !(b - 1);
    for i in 0..16 {
        let t = c & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}

fn pack25519(n: &Gf) -> [u8; 32] {
    let mut t = *n;
    car25519(&mut t);
    car25519(&mut t);
    car25519(&mut t);
    let mut m = GF0;
    for _ in 0..2 {
        m[0] = t[0] - 0xffed;
        for i in 1..15 {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        let b = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        sel25519(&mut t, &mut m, 1 - b);
    }
    let mut o = [0u8; 32];
    for i in 0..16 {
        o[2 * i] = (t[i] & 0xff) as u8;
        o[2 * i + 1] = (t[i] >> 8) as u8;
    }
    o
}

fn neq25519(a: &Gf, b: &Gf) -> bool {
    !ct_eq(&pack25519(a), &pack25519(b))
}

fn par25519(a: &Gf) -> u8 {
    pack25519(a)[0] & 1
}

fn unpack25519(n: &[u8; 32]) -> Gf {
    let mut o = GF0;
    for i in 0..16 {
        o[i] = n[2 * i] as i64 + ((n[2 * i + 1] as i64) << 8);
    }
    o[15] &= 0x7fff;
    o
}

fn add_gf(a: &Gf, b: &Gf) -> Gf {
    let mut o = GF0;
    for i in 0..16 {
        o[i] = a[i] + b[i];
    }
    o
}

fn sub_gf(a: &Gf, b: &Gf) -> Gf {
    let mut o = GF0;
    for i in 0..16 {
        o[i] = a[i] - b[i];
    }
    o
}

fn mul_gf(a: &Gf, b: &Gf) -> Gf {
    let mut t = [0i64; 31];
    for i in 0..16 {
        for j in 0..16 {
            t[i + j] += a[i] * b[j];
        }
    }
    for i in 0..15 {
        t[i] += 38 * t[i + 16];
    }
    let mut o = GF0;
    o.copy_from_slice(&t[..16]);
    car25519(&mut o);
    car25519(&mut o);
    o
}

fn sq_gf(a: &Gf) -> Gf {
    mul_gf(a, a)
}

fn inv25519(i: &Gf) -> Gf {
    let mut c = *i;
    for a in (0..=253).rev() {
        c = sq_gf(&c);
        if a != 2 && a != 4 {
            c = mul_gf(&c, i);
        }
    }
    c
}

fn pow2523(i: &Gf) -> Gf {
    let mut c = *i;
    for a in (0..=250).rev() {
        c = sq_gf(&c);
        if a != 1 {
            c = mul_gf(&c, i);
        }
    }
    c
}

type Point = [Gf; 4];

fn add(p: &mut Point, q: &Point) {
    let a = mul_gf(&sub_gf(&p[1], &p[0]), &sub_gf(&q[1], &q[0]));
    let b = mul_gf(&add_gf(&p[0], &p[1]), &add_gf(&q[0], &q[1]));
    let c = mul_gf(&mul_gf(&p[3], &q[3]), &D2);
    let d = mul_gf(&p[2], &q[2]);
    let d = add_gf(&d, &d);
    let e = sub_gf(&b, &a);
    let f = sub_gf(&d, &c);
    let g = add_gf(&d, &c);
    let h = add_gf(&b, &a);
    p[0] = mul_gf(&e, &f);
    p[1] = mul_gf(&h, &g);
    p[2] = mul_gf(&g, &f);
    p[3] = mul_gf(&e, &h);
}

fn cswap(p: &mut Point, q: &mut Point, b: u8) {
    for i in 0..4 {
        sel25519(&mut p[i], &mut q[i], b as i64);
    }
}

fn pack(p: &Point) -> [u8; 32] {
    let zi = inv25519(&p[2]);
    let tx = mul_gf(&p[0], &zi);
    let ty = mul_gf(&p[1], &zi);
    let mut r = pack25519(&ty);
    r[31] ^= par25519(&tx) << 7;
    r
}

fn scalarmult(q: &Point, s: &[u8; 32]) -> Point {
    let mut p: Point = [GF0, GF1, GF1, GF0];
    let mut q = *q;
    for i in (0..256).rev() {
        let b = (s[i / 8] >> (i & 7)) & 1;
        cswap(&mut p, &mut q, b);
        add(&mut q, &p);
        let pp = p;
        add(&mut p, &pp);
        cswap(&mut p, &mut q, b);
    }
    p
}

fn scalarbase(s: &[u8; 32]) -> Point {
    let q: Point = [X, Y, GF1, mul_gf(&X, &Y)];
    scalarmult(&q, s)
}

fn mod_l(x: &mut [i64; 64]) -> [u8; 32] {
    for i in (32..64).rev() {
        let mut carry = 0;
        let mut j = i - 32;
        while j < i - 12 {
            x[j] += carry - 16 * x[i] * L[j - (i - 32)];
            carry = (x[j] + 128) >> 8;
            x[j] -= carry << 8;
            j += 1;
        }
        x[j] += carry;
        x[i] = 0;
    }
    let mut carry = 0;
    for j in 0..32 {
        x[j] += carry - (x[31] >> 4) * L[j];
        carry = x[j] >> 8;
        x[j] &= 255;
    }
    for j in 0..32 {
        x[j] -= carry * L[j];
    }
    let mut r = [0u8; 32];
    for i in 0..32 {
        x[i + 1] += x[i] >> 8;
        r[i] = (x[i] & 255) as u8;
    }
    r
}

fn reduce(h: &[u8; 64]) -> [u8; 32] {
    let mut x = [0i64; 64];
    for i in 0..64 {
        x[i] = h[i] as i64;
    }
    mod_l(&mut x)
}

/// `-A` from an encoded point `A` (verification adds `-h·A`), or `None`
/// when the bytes are not a curve point.
fn unpackneg(p: &[u8; 32]) -> Option<Point> {
    let mut r: Point = [GF0, GF0, GF1, GF0];
    r[1] = unpack25519(p);
    let num = sq_gf(&r[1]);
    let den = mul_gf(&num, &D);
    let num = sub_gf(&num, &r[2]);
    let den = add_gf(&r[2], &den);
    let den2 = sq_gf(&den);
    let den4 = sq_gf(&den2);
    let den6 = mul_gf(&den4, &den2);
    let mut t = mul_gf(&den6, &num);
    t = mul_gf(&t, &den);
    t = pow2523(&t);
    t = mul_gf(&t, &num);
    t = mul_gf(&t, &den);
    t = mul_gf(&t, &den);
    r[0] = mul_gf(&t, &den);
    let mut chk = mul_gf(&sq_gf(&r[0]), &den);
    if neq25519(&chk, &num) {
        r[0] = mul_gf(&r[0], &I);
    }
    chk = mul_gf(&sq_gf(&r[0]), &den);
    if neq25519(&chk, &num) {
        return None;
    }
    if par25519(&r[0]) == (p[31] >> 7) {
        r[0] = sub_gf(&GF0, &r[0]);
    }
    r[3] = mul_gf(&r[0], &r[1]);
    Some(r)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The clamped secret scalar and the nonce prefix of a seed.
fn expand(seed: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let d = sha512::digest(&[seed]);
    let mut a = [0u8; 32];
    a.copy_from_slice(&d[..32]);
    a[0] &= 248;
    a[31] &= 127;
    a[31] |= 64;
    let mut prefix = [0u8; 32];
    prefix.copy_from_slice(&d[32..]);
    (a, prefix)
}

/// The public key of a 32-byte secret seed.
pub fn public_key(seed: &[u8; SEED_LEN]) -> [u8; PUBLIC_KEY_LEN] {
    let (a, _) = expand(seed);
    pack(&scalarbase(&a))
}

/// Signs `message` with the key pair of `seed` (deterministic, RFC 8032).
pub fn sign(seed: &[u8; SEED_LEN], message: &[u8]) -> [u8; SIGNATURE_LEN] {
    let (a, prefix) = expand(seed);
    let pk = pack(&scalarbase(&a));
    let r = reduce(&sha512::digest(&[&prefix, message]));
    let big_r = pack(&scalarbase(&r));
    let h = reduce(&sha512::digest(&[&big_r, &pk, message]));
    let mut x = [0i64; 64];
    for i in 0..32 {
        x[i] = r[i] as i64;
    }
    for i in 0..32 {
        for j in 0..32 {
            x[i + j] += h[i] as i64 * a[j] as i64;
        }
    }
    let s = mod_l(&mut x);
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&big_r);
    sig[32..].copy_from_slice(&s);
    sig
}

/// `S < L` (little endian).
fn scalar_is_canonical(s: &[u8]) -> bool {
    for i in (0..32).rev() {
        let l = L[i] as u8;
        if s[i] < l {
            return true;
        }
        if s[i] > l {
            return false;
        }
    }
    false
}

/// Whether `signature` is a valid signature of `message` by `public_key`.
pub fn verify(
    public_key: &[u8; PUBLIC_KEY_LEN],
    message: &[u8],
    signature: &[u8; SIGNATURE_LEN],
) -> bool {
    if !scalar_is_canonical(&signature[32..]) {
        return false;
    }
    let Some(neg_a) = unpackneg(public_key) else {
        return false;
    };
    let h = reduce(&sha512::digest(&[&signature[..32], public_key, message]));
    let mut p = scalarmult(&neg_a, &h);
    let mut s = [0u8; 32];
    s.copy_from_slice(&signature[32..]);
    let q = scalarbase(&s);
    add(&mut p, &q);
    ct_eq(&pack(&p), &signature[..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{from_hex, hex};

    /// (seed, public key, message, signature): RFC 8032 section 7.1 tests 1
    /// and 2, then vectors from another implementation (Python
    /// `cryptography`).
    const VECTORS: &[(&str, &str, &str, &str)] = &[
        (
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        ),
        (
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            "72",
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        ),
        (
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8",
            "",
            "9ca53579530654d5c3df77089ef45eda613e2fedf670e96bedac4639504e5845ef4b95d5793077233dd16817b2532e9c5525872a73a4ad74b759369a9e05c102",
        ),
        (
            "0707070707070707070707070707070707070707070707070707070707070707",
            "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c",
            "6f726269742d706c7567696e2d6275696c642f76310a69643d61636d652e68656c6c6f0a",
            "eb4aa0965a755aefb00882b374d83fbfdf0b911eb9a257c2c513dd91ab5d80b8dd118658c695a91fce11f9617554dacec18265fb4e5f38827f827428252bb90c",
        ),
    ];

    fn bytes(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn known_vectors_match() {
        for (seed, pk, msg, sig) in VECTORS {
            let seed = from_hex::<32>(seed).unwrap();
            assert_eq!(hex(&public_key(&seed)), *pk);
            let msg = bytes(msg);
            assert_eq!(hex(&sign(&seed, &msg)), *sig);
            assert!(verify(
                &from_hex(pk).unwrap(),
                &msg,
                &from_hex(sig).unwrap()
            ));
        }
        // A long message (768 bytes), signed by Python `cryptography`.
        let seed =
            from_hex::<32>("4fa1a13ac468ac495f3390e859d76d5e8ef49806815b45a21de7711bcc624194")
                .unwrap();
        let msg: Vec<u8> = (0..=255u8).cycle().take(768).collect();
        let sig = "2851b6d7df81d9dde1b73ebdfc7bd770ab8dd936da216e57d1790218a1627f7a34661fecd2596910a303230bbc9f6a81207bc6c9e3934c8a31ad7424ff051609";
        assert_eq!(
            hex(&public_key(&seed)),
            "36c754b81513791d9bbf9fd74c9af1f2cb95fee5f1715f8c8b0c22cf246c036d"
        );
        assert_eq!(hex(&sign(&seed, &msg)), sig);
    }

    #[test]
    fn tampering_is_detected() {
        let seed = [42u8; 32];
        let pk = public_key(&seed);
        let msg = b"orbit-plugin-build/v1\nid=acme.hello\n";
        let sig = sign(&seed, msg);
        assert!(verify(&pk, msg, &sig));
        // Another message.
        assert!(!verify(
            &pk,
            b"orbit-plugin-build/v1\nid=acme.hellO\n",
            &sig
        ));
        // Every bit of the signature matters.
        for byte in [0, 17, 31, 32, 50, 63] {
            let mut bad = sig;
            bad[byte] ^= 1;
            assert!(!verify(&pk, msg, &bad), "flipped byte {byte}");
        }
        // Another key.
        assert!(!verify(&public_key(&[43u8; 32]), msg, &sig));
        // A non-canonical S (S + L) is refused.
        let mut s = [0i64; 64];
        for i in 0..32 {
            s[i] = sig[32 + i] as i64 + L[i];
        }
        let mut carry = 0;
        let mut malleable = sig;
        for i in 0..32 {
            let v = s[i] + carry;
            malleable[32 + i] = (v & 255) as u8;
            carry = v >> 8;
        }
        assert_eq!(carry, 0);
        assert!(!verify(&pk, msg, &malleable));
        // Not a point.
        let mut not_point = [0u8; 32];
        not_point[0] = 2;
        assert!(!verify(&not_point, msg, &sig));
    }
}
