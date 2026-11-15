//! Byte-level Windows security descriptor rules for the token file.
//!
//! This module is plain safe Rust so the rules can be tested on every platform.
//! The Windows system calls that produce and consume these bytes live in
//! `windows.rs`.

/// `S-1-5-18`, the local system account.
const SYSTEM_SID: &[u8] = &[1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
/// `S-1-5-32-544`, the built-in Administrators group.
const ADMINISTRATORS_SID: &[u8] = &[1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 32, 2, 0, 0];

const ACL_REVISION: u8 = 2;
const ACL_HEADER_LEN: usize = 8;
const ACE_HEADER_LEN: usize = 4;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
/// `FILE_ALL_ACCESS`.
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

const DESCRIPTOR_HEADER_LEN: usize = 20;
const SE_DACL_PRESENT: u16 = 0x0004;
const SE_DACL_PROTECTED: u16 = 0x1000;
const SE_SELF_RELATIVE: u16 = 0x8000;

/// Returns the length of the SID at the start of `bytes`, if it is well formed.
pub(super) fn sid_len(bytes: &[u8]) -> Option<usize> {
    let revision = *bytes.first()?;
    let count = usize::from(*bytes.get(1)?);
    if revision != 1 || count == 0 || count > 15 {
        return None;
    }
    let length = 8 + 4 * count;
    (bytes.len() >= length).then_some(length)
}

/// Builds an ACL that grants full file access to `user` and to nobody else.
pub(super) fn owner_only_acl(user: &[u8]) -> Option<Vec<u8>> {
    let sid_length = sid_len(user).filter(|length| *length == user.len())?;
    let entry_length = ACE_HEADER_LEN + 4 + sid_length;
    let acl_length = u16::try_from(ACL_HEADER_LEN + entry_length).ok()?;
    let entry_length = u16::try_from(entry_length).ok()?;
    let mut acl = Vec::with_capacity(usize::from(acl_length));
    acl.extend_from_slice(&[ACL_REVISION, 0]);
    acl.extend_from_slice(&acl_length.to_le_bytes());
    acl.extend_from_slice(&1_u16.to_le_bytes());
    acl.extend_from_slice(&0_u16.to_le_bytes());
    acl.extend_from_slice(&[ACCESS_ALLOWED_ACE_TYPE, 0]);
    acl.extend_from_slice(&entry_length.to_le_bytes());
    acl.extend_from_slice(&FILE_ALL_ACCESS.to_le_bytes());
    acl.extend_from_slice(user);
    Some(acl)
}

/// Builds a self-relative descriptor with an owner-only, protected DACL.
/// Passing it at file creation prevents inherited permissions from exposing
/// the file, even before the first token byte is written.
pub(super) fn private_descriptor(user: &[u8]) -> Option<Vec<u8>> {
    let acl = owner_only_acl(user)?;
    let mut descriptor = vec![1, 0];
    descriptor
        .extend_from_slice(&(SE_SELF_RELATIVE | SE_DACL_PRESENT | SE_DACL_PROTECTED).to_le_bytes());
    descriptor.extend_from_slice(&u32::try_from(DESCRIPTOR_HEADER_LEN).ok()?.to_le_bytes());
    descriptor.extend_from_slice(&[0; 8]); // No primary group or SACL.
    descriptor.extend_from_slice(
        &u32::try_from(DESCRIPTOR_HEADER_LEN + user.len())
            .ok()?
            .to_le_bytes(),
    );
    descriptor.extend_from_slice(user);
    descriptor.extend_from_slice(&acl);
    Some(descriptor)
}

/// Checks that a self-relative security descriptor keeps a file private to
/// `user`.
///
/// The owner and every access-allowed entry must name `user`, the local system
/// account, or the built-in Administrators group, which can read any file
/// anyway. Access-denied entries are allowed; any other entry type, a missing
/// owner, or a missing (allow-everyone) DACL is rejected.
pub(super) fn check_private(descriptor: &[u8], user: &[u8]) -> Result<(), &'static str> {
    const MALFORMED: &str = "its security descriptor is malformed";
    if descriptor.len() < DESCRIPTOR_HEADER_LEN || descriptor[0] != 1 {
        return Err(MALFORMED);
    }
    let control = u16::from_le_bytes([descriptor[2], descriptor[3]]);
    if control & SE_SELF_RELATIVE == 0 {
        return Err(MALFORMED);
    }
    let owner_offset = read_u32(descriptor, 4).ok_or(MALFORMED)?;
    let dacl_offset = read_u32(descriptor, 16).ok_or(MALFORMED)?;
    if owner_offset == 0 {
        return Err("it has no owner");
    }
    let owner = sid_at(descriptor, owner_offset).ok_or(MALFORMED)?;
    if !trusted(owner, user) {
        return Err("it is owned by another account");
    }
    if control & SE_DACL_PRESENT == 0 || dacl_offset == 0 {
        return Err("it has no access list, which grants everyone access");
    }
    let acl = descriptor.get(dacl_offset..).ok_or(MALFORMED)?;
    if acl.len() < ACL_HEADER_LEN {
        return Err(MALFORMED);
    }
    if !matches!(acl[0], ACL_REVISION | 4) {
        return Err(MALFORMED);
    }
    let acl_length = usize::from(u16::from_le_bytes([acl[2], acl[3]]));
    if acl_length < ACL_HEADER_LEN {
        return Err(MALFORMED);
    }
    let count = u16::from_le_bytes([acl[4], acl[5]]);
    let acl = acl.get(..acl_length).ok_or(MALFORMED)?;
    let mut offset = ACL_HEADER_LEN;
    for _ in 0..count {
        let header = acl.get(offset..offset + ACE_HEADER_LEN).ok_or(MALFORMED)?;
        let entry_length = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if entry_length < ACE_HEADER_LEN {
            return Err(MALFORMED);
        }
        let ace = acl.get(offset..offset + entry_length).ok_or(MALFORMED)?;
        match header[0] {
            ACCESS_DENIED_ACE_TYPE => {}
            ACCESS_ALLOWED_ACE_TYPE => {
                let sid = ace.get(ACE_HEADER_LEN + 4..).ok_or(MALFORMED)?;
                let sid = sid.get(..sid_len(sid).ok_or(MALFORMED)?).ok_or(MALFORMED)?;
                if !trusted(sid, user) {
                    return Err("its access list grants access to another account");
                }
            }
            _ => return Err("its access list has an unsupported entry"),
        }
        offset += entry_length;
    }
    Ok(())
}

fn trusted(sid: &[u8], user: &[u8]) -> bool {
    sid == user || sid == SYSTEM_SID || sid == ADMINISTRATORS_SID
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<usize> {
    let value = bytes.get(offset..offset + 4)?;
    usize::try_from(u32::from_le_bytes([value[0], value[1], value[2], value[3]])).ok()
}

fn sid_at(bytes: &[u8], offset: usize) -> Option<&[u8]> {
    let bytes = bytes.get(offset..)?;
    bytes.get(..sid_len(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::{
        ADMINISTRATORS_SID, SYSTEM_SID, check_private, owner_only_acl, private_descriptor, sid_len,
    };

    /// `S-1-5-21-1-2-3-1001`.
    const USER: &[u8] = &[
        1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0xE9, 3, 0, 0,
    ];
    /// `S-1-5-21-1-2-3-1002`.
    const OTHER: &[u8] = &[
        1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0xEA, 3, 0, 0,
    ];
    /// `S-1-1-0`, Everyone.
    const EVERYONE: &[u8] = &[1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];

    fn ace(kind: u8, sid: &[u8]) -> Vec<u8> {
        let length = u16::try_from(8 + sid.len()).unwrap_or_else(|error| panic!("{error}"));
        let mut ace = vec![kind, 0];
        ace.extend_from_slice(&length.to_le_bytes());
        ace.extend_from_slice(&0x0012_0089_u32.to_le_bytes());
        ace.extend_from_slice(sid);
        ace
    }

    fn acl(aces: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = aces.concat();
        let length = u16::try_from(8 + body.len()).unwrap_or_else(|error| panic!("{error}"));
        let count = u16::try_from(aces.len()).unwrap_or_else(|error| panic!("{error}"));
        let mut acl = vec![2, 0];
        acl.extend_from_slice(&length.to_le_bytes());
        acl.extend_from_slice(&count.to_le_bytes());
        acl.extend_from_slice(&[0, 0]);
        acl.extend_from_slice(&body);
        acl
    }

    fn descriptor(owner: Option<&[u8]>, dacl: Option<&[u8]>) -> Vec<u8> {
        let mut control = 0x8000_u16;
        let mut tail = Vec::new();
        let owner_offset = owner.map_or(0, |sid| {
            let offset = 20 + tail.len();
            tail.extend_from_slice(sid);
            offset
        });
        let dacl_offset = dacl.map_or(0, |acl| {
            control |= 0x0004;
            let offset = 20 + tail.len();
            tail.extend_from_slice(acl);
            offset
        });
        let offset = |value: usize| {
            u32::try_from(value)
                .unwrap_or_else(|error| panic!("{error}"))
                .to_le_bytes()
        };
        let mut bytes = vec![1, 0];
        bytes.extend_from_slice(&control.to_le_bytes());
        bytes.extend_from_slice(&offset(owner_offset));
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&offset(dacl_offset));
        bytes.extend_from_slice(&tail);
        bytes
    }

    #[test]
    fn owner_only_acl_round_trips_through_the_privacy_check() {
        let acl = owner_only_acl(USER).unwrap_or_else(|| panic!("ACL was not built"));
        assert_eq!(acl.len(), 8 + 8 + USER.len());
        assert_eq!(acl.len() % 4, 0);
        assert_eq!(
            check_private(&descriptor(Some(USER), Some(&acl)), USER),
            Ok(())
        );
        assert!(check_private(&descriptor(Some(USER), Some(&acl)), OTHER).is_err());
        let private =
            private_descriptor(USER).unwrap_or_else(|| panic!("descriptor was not built"));
        assert_eq!(check_private(&private, USER), Ok(()));
        assert_eq!(private[3] & 0x10, 0x10, "inheritance must be disabled");
    }

    #[test]
    fn owner_only_acl_rejects_malformed_sids() {
        assert!(owner_only_acl(&USER[..USER.len() - 1]).is_none());
        assert!(owner_only_acl(&[USER, &[0]].concat()).is_none());
        assert!(owner_only_acl(&[]).is_none());
        assert_eq!(sid_len(EVERYONE), Some(12));
    }

    #[test]
    fn accepts_system_administrators_and_deny_entries() {
        let dacl = acl(&[
            ace(1, EVERYONE),
            ace(0, USER),
            ace(0, SYSTEM_SID),
            ace(0, ADMINISTRATORS_SID),
        ]);
        assert_eq!(
            check_private(&descriptor(Some(ADMINISTRATORS_SID), Some(&dacl)), USER),
            Ok(())
        );
    }

    #[test]
    fn rejects_access_for_other_accounts() {
        for sid in [OTHER, EVERYONE] {
            let dacl = acl(&[ace(0, USER), ace(0, sid)]);
            assert_eq!(
                check_private(&descriptor(Some(USER), Some(&dacl)), USER),
                Err("its access list grants access to another account")
            );
        }
        let dacl = acl(&[ace(0, USER)]);
        assert_eq!(
            check_private(&descriptor(Some(OTHER), Some(&dacl)), USER),
            Err("it is owned by another account")
        );
    }

    #[test]
    fn rejects_null_dacls_missing_owners_and_unknown_entries() {
        let dacl = acl(&[ace(0, USER)]);
        assert_eq!(
            check_private(&descriptor(Some(USER), None), USER),
            Err("it has no access list, which grants everyone access")
        );
        assert_eq!(
            check_private(&descriptor(None, Some(&dacl)), USER),
            Err("it has no owner")
        );
        let callback = acl(&[ace(9, USER)]);
        assert_eq!(
            check_private(&descriptor(Some(USER), Some(&callback)), USER),
            Err("its access list has an unsupported entry")
        );
    }

    #[test]
    fn rejects_truncated_descriptors() {
        let dacl = acl(&[ace(0, USER)]);
        let full = descriptor(Some(USER), Some(&dacl));
        for length in 0..full.len() {
            assert!(check_private(&full[..length], USER).is_err(), "{length}");
        }
        let mut zero_length_ace = acl(&[ace(0, USER)]);
        zero_length_ace[10] = 0;
        zero_length_ace[11] = 0;
        assert!(check_private(&descriptor(Some(USER), Some(&zero_length_ace)), USER).is_err());
    }
}
