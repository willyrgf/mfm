//! Exact-target PostgreSQL role naming for one store schema.
//!
//! Role names are assigned by the destructive baseline and retained in
//! `target_authority`. Callers never invent sibling-target role names.

/// Managed role kinds granted exclusively to one store target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TargetRoleKind {
    Owner,
    Qualification,
    RunReader,
    RunWriter,
    ConfigurationReader,
    ConfigurationWriter,
}

impl TargetRoleKind {
    pub(crate) const fn suffix(self) -> &'static str {
        match self {
            Self::Owner => "own",
            Self::Qualification => "qlf",
            Self::RunReader => "rrd",
            Self::RunWriter => "rwr",
            Self::ConfigurationReader => "crd",
            Self::ConfigurationWriter => "cwr",
        }
    }

    pub(crate) const fn catalog_token(self) -> &'static str {
        match self {
            Self::Owner => "<target-owner>",
            Self::Qualification => "<target-qualification>",
            Self::RunReader => "<target-run-reader>",
            Self::RunWriter => "<target-run-writer>",
            Self::ConfigurationReader => "<target-configuration-reader>",
            Self::ConfigurationWriter => "<target-configuration-writer>",
        }
    }

    pub(crate) const fn all() -> [Self; 6] {
        [
            Self::Owner,
            Self::Qualification,
            Self::RunReader,
            Self::RunWriter,
            Self::ConfigurationReader,
            Self::ConfigurationWriter,
        ]
    }
}

/// Validated 16-hex target key used to name schema-local roles.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TargetKey(String);

impl TargetKey {
    /// Derives the same target key the destructive baseline stores for `schema_name`.
    ///
    /// Matches PostgreSQL `substr(md5(schema_name), 1, 16)`.
    pub(crate) fn from_schema(schema_name: &str) -> Self {
        Self(md5_hex(schema_name.as_bytes())[..16].to_owned())
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.len() == 16
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            Some(Self(value.to_owned()))
        } else {
            None
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn role_name(&self, kind: TargetRoleKind) -> String {
        format!("mfm_t_{}_{}", self.0, kind.suffix())
    }
}

/// RFC 1321 MD5 used only to mirror PostgreSQL's built-in `md5(text)` naming helper.
fn md5_hex(input: &[u8]) -> String {
    fn f(x: u32, y: u32, z: u32) -> u32 {
        (x & y) | (!x & z)
    }
    fn g(x: u32, y: u32, z: u32) -> u32 {
        (x & z) | (y & !z)
    }
    fn h(x: u32, y: u32, z: u32) -> u32 {
        x ^ y ^ z
    }
    fn i(x: u32, y: u32, z: u32) -> u32 {
        y ^ (x | !z)
    }
    fn op(a: u32, b: u32, c: u32, d: u32, x: u32, s: u32, ac: u32, func: fn(u32, u32, u32) -> u32) -> u32 {
        a.wrapping_add(func(b, c, d))
            .wrapping_add(x)
            .wrapping_add(ac)
            .rotate_left(s)
            .wrapping_add(b)
    }

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut msg = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let mut state = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in msg.chunks_exact(64) {
        let mut x = [0u32; 16];
        for (idx, word) in x.iter_mut().enumerate() {
            let base = idx * 4;
            *word = u32::from_le_bytes([
                chunk[base],
                chunk[base + 1],
                chunk[base + 2],
                chunk[base + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
        a = op(a, b, c, d, x[0], 7, 0xd76a_a478, f);
        d = op(d, a, b, c, x[1], 12, 0xe8c7_b756, f);
        c = op(c, d, a, b, x[2], 17, 0x2420_70db, f);
        b = op(b, c, d, a, x[3], 22, 0xc1bd_ceee, f);
        a = op(a, b, c, d, x[4], 7, 0xf57c_0faf, f);
        d = op(d, a, b, c, x[5], 12, 0x4787_c62a, f);
        c = op(c, d, a, b, x[6], 17, 0xa830_4613, f);
        b = op(b, c, d, a, x[7], 22, 0xfd46_9501, f);
        a = op(a, b, c, d, x[8], 7, 0x6980_98d8, f);
        d = op(d, a, b, c, x[9], 12, 0x8b44_f7af, f);
        c = op(c, d, a, b, x[10], 17, 0xffff_5bb1, f);
        b = op(b, c, d, a, x[11], 22, 0x895c_d7be, f);
        a = op(a, b, c, d, x[12], 7, 0x6b90_1122, f);
        d = op(d, a, b, c, x[13], 12, 0xfd98_7193, f);
        c = op(c, d, a, b, x[14], 17, 0xa679_438e, f);
        b = op(b, c, d, a, x[15], 22, 0x49b4_0821, f);

        a = op(a, b, c, d, x[1], 5, 0xf61e_2562, g);
        d = op(d, a, b, c, x[6], 9, 0xc040_b340, g);
        c = op(c, d, a, b, x[11], 14, 0x265e_5a51, g);
        b = op(b, c, d, a, x[0], 20, 0xe9b6_c7aa, g);
        a = op(a, b, c, d, x[5], 5, 0xd62f_105d, g);
        d = op(d, a, b, c, x[10], 9, 0x0244_1453, g);
        c = op(c, d, a, b, x[15], 14, 0xd8a1_e681, g);
        b = op(b, c, d, a, x[4], 20, 0xe7d3_fbc8, g);
        a = op(a, b, c, d, x[9], 5, 0x21e1_cde6, g);
        d = op(d, a, b, c, x[14], 9, 0xc337_07d6, g);
        c = op(c, d, a, b, x[3], 14, 0xf4d5_0d87, g);
        b = op(b, c, d, a, x[8], 20, 0x455a_14ed, g);
        a = op(a, b, c, d, x[13], 5, 0xa9e3_e905, g);
        d = op(d, a, b, c, x[2], 9, 0xfcef_a3f8, g);
        c = op(c, d, a, b, x[7], 14, 0x676f_02d9, g);
        b = op(b, c, d, a, x[12], 20, 0x8d2a_4c8a, g);

        a = op(a, b, c, d, x[5], 4, 0xfffa_3942, h);
        d = op(d, a, b, c, x[8], 11, 0x8771_f681, h);
        c = op(c, d, a, b, x[11], 16, 0x6d9d_6122, h);
        b = op(b, c, d, a, x[14], 23, 0xfde5_380c, h);
        a = op(a, b, c, d, x[1], 4, 0xa4be_ea44, h);
        d = op(d, a, b, c, x[4], 11, 0x4bde_cfa9, h);
        c = op(c, d, a, b, x[7], 16, 0xf6bb_4b60, h);
        b = op(b, c, d, a, x[10], 23, 0xbebf_bc70, h);
        a = op(a, b, c, d, x[13], 4, 0x289b_7ec6, h);
        d = op(d, a, b, c, x[0], 11, 0xeaa1_27fa, h);
        c = op(c, d, a, b, x[3], 16, 0xd4ef_3085, h);
        b = op(b, c, d, a, x[6], 23, 0x0488_1d05, h);
        a = op(a, b, c, d, x[9], 4, 0xd9d4_d039, h);
        d = op(d, a, b, c, x[12], 11, 0xe6db_99e5, h);
        c = op(c, d, a, b, x[15], 16, 0x1fa2_7cf8, h);
        b = op(b, c, d, a, x[2], 23, 0xc4ac_5665, h);

        a = op(a, b, c, d, x[0], 6, 0xf429_2244, i);
        d = op(d, a, b, c, x[7], 10, 0x432a_ff97, i);
        c = op(c, d, a, b, x[14], 15, 0xab94_23a7, i);
        b = op(b, c, d, a, x[5], 21, 0xfc93_a039, i);
        a = op(a, b, c, d, x[12], 6, 0x655b_59c3, i);
        d = op(d, a, b, c, x[3], 10, 0x8f0c_cc92, i);
        c = op(c, d, a, b, x[10], 15, 0xffef_f47d, i);
        b = op(b, c, d, a, x[1], 21, 0x8584_5dd1, i);
        a = op(a, b, c, d, x[8], 6, 0x6fa8_7e4f, i);
        d = op(d, a, b, c, x[15], 10, 0xfe2c_e6e0, i);
        c = op(c, d, a, b, x[6], 15, 0xa301_4314, i);
        b = op(b, c, d, a, x[13], 21, 0x4e08_11a1, i);
        a = op(a, b, c, d, x[4], 6, 0xf753_7e82, i);
        d = op(d, a, b, c, x[11], 10, 0xbd3a_f235, i);
        c = op(c, d, a, b, x[2], 15, 0x2ad7_d2bb, i);
        b = op(b, c, d, a, x[9], 21, 0xeb86_d391, i);

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    let mut out = String::with_capacity(32);
    for word in state {
        for byte in word.to_le_bytes() {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
    }
    out
}

/// Exact retained role set for one store target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetRoleNames {
    pub(crate) target_key: TargetKey,
    pub(crate) owner: String,
    pub(crate) qualification: String,
    pub(crate) run_reader: String,
    pub(crate) run_writer: String,
    pub(crate) configuration_reader: String,
    pub(crate) configuration_writer: String,
}

impl TargetRoleNames {
    pub(crate) fn from_target_key(target_key: TargetKey) -> Self {
        Self {
            owner: target_key.role_name(TargetRoleKind::Owner),
            qualification: target_key.role_name(TargetRoleKind::Qualification),
            run_reader: target_key.role_name(TargetRoleKind::RunReader),
            run_writer: target_key.role_name(TargetRoleKind::RunWriter),
            configuration_reader: target_key.role_name(TargetRoleKind::ConfigurationReader),
            configuration_writer: target_key.role_name(TargetRoleKind::ConfigurationWriter),
            target_key,
        }
    }

    pub(crate) fn name(&self, kind: TargetRoleKind) -> &str {
        match kind {
            TargetRoleKind::Owner => &self.owner,
            TargetRoleKind::Qualification => &self.qualification,
            TargetRoleKind::RunReader => &self.run_reader,
            TargetRoleKind::RunWriter => &self.run_writer,
            TargetRoleKind::ConfigurationReader => &self.configuration_reader,
            TargetRoleKind::ConfigurationWriter => &self.configuration_writer,
        }
    }

    pub(crate) fn managed_names(&self) -> [&str; 6] {
        [
            self.owner.as_str(),
            self.qualification.as_str(),
            self.run_reader.as_str(),
            self.run_writer.as_str(),
            self.configuration_reader.as_str(),
            self.configuration_writer.as_str(),
        ]
    }
}

/// Maps a managed role name to its catalog token for schema-independent hashing.
pub(crate) fn normalize_role_name(role_name: &str) -> String {
    if let Some((key_part, suffix)) = role_name
        .strip_prefix("mfm_t_")
        .and_then(|rest| rest.split_once('_'))
    {
        if let Some(key) = TargetKey::parse(key_part) {
            for kind in TargetRoleKind::all() {
                if suffix == kind.suffix() && role_name == key.role_name(kind) {
                    return kind.catalog_token().to_owned();
                }
            }
        }
    }
    role_name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{md5_hex, normalize_role_name, TargetKey, TargetRoleKind, TargetRoleNames};

    #[test]
    fn target_key_roles_are_stable() {
        let key = TargetKey::parse("0123456789abcdef").expect("key");
        let names = TargetRoleNames::from_target_key(key.clone());
        assert_eq!(names.run_writer, "mfm_t_0123456789abcdef_rwr");
        assert_eq!(
            normalize_role_name(&key.role_name(TargetRoleKind::Owner)),
            "<target-owner>"
        );
        assert_eq!(normalize_role_name("postgres"), "postgres");
    }

    #[test]
    fn md5_matches_known_vector() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            TargetKey::from_schema("mfm_structured_example").as_str(),
            &md5_hex(b"mfm_structured_example")[..16]
        );
    }
}
