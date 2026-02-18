pub fn trim_leading_zero_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx] == 0 {
        idx += 1;
    }
    bytes[idx..].to_vec()
}

pub fn u64_to_min_be(mut value: u64) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

pub fn u128_to_min_be(mut value: u128) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

pub fn usize_to_min_be(mut value: usize) -> Vec<u8> {
    if value == 0 {
        return vec![0];
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

pub fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }

    let mut out = Vec::new();
    if bytes.len() <= 55 {
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        return out;
    }

    let len_bytes = usize_to_min_be(bytes.len());
    out.push(0xb7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(bytes);
    out
}

pub fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(&rlp_encode_bytes(item));
    }

    let mut out = Vec::new();
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(&payload);
        return out;
    }

    let len_bytes = usize_to_min_be(payload.len());
    out.push(0xf7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(&payload);
    out
}

pub fn rlp_encode_list_preencoded(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(item);
    }

    let mut out = Vec::new();
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(&payload);
        return out;
    }

    let len_bytes = usize_to_min_be(payload.len());
    out.push(0xf7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(&payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlp_zero_length_list_is_c0() {
        assert_eq!(rlp_encode_list_preencoded(&[]), vec![0xc0]);
    }

    #[test]
    fn usize_zero_is_single_zero_byte() {
        assert_eq!(usize_to_min_be(0), vec![0]);
    }

    #[test]
    fn u64_zero_is_empty() {
        assert_eq!(u64_to_min_be(0), Vec::<u8>::new());
    }
}
