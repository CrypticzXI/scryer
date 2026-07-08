use std::mem::MaybeUninit;

use extism::{CurrentPlugin, Error, Function, UserData, Val, ValType, convert::MemoryHandle};

const ARCHIVE_CRYPTO_HOST_NAMESPACE: &str = "extism:host/user";
const AES_BLOCK_LEN: usize = 16;
const AES_128_KEY_LEN: usize = 16;
const AES_256_KEY_LEN: usize = 32;
const AES_STATUS_OK: i64 = 0;
const AES_STATUS_BAD_KEY_LEN: i64 = -1;
const AES_STATUS_BAD_BLOCK_LEN: i64 = -2;
const AES_STATUS_OUT_OF_BOUNDS: i64 = -3;
const CRC_STATUS_OUT_OF_BOUNDS: i64 = -1;

pub(crate) fn functions() -> Vec<Function> {
    vec![
        Function::new(
            "scryer_aes_cbc_decrypt",
            [
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
            ],
            [ValType::I64],
            UserData::new(()),
            scryer_aes_cbc_decrypt,
        )
        .with_namespace(ARCHIVE_CRYPTO_HOST_NAMESPACE),
        Function::new(
            "scryer_crc32",
            [ValType::I64, ValType::I64, ValType::I64],
            [ValType::I64],
            UserData::new(()),
            scryer_crc32,
        )
        .with_namespace(ARCHIVE_CRYPTO_HOST_NAMESPACE),
    ]
}

fn scryer_aes_cbc_decrypt(
    current: &mut CurrentPlugin,
    input: &[Val],
    output: &mut [Val],
    _state: UserData<()>,
) -> Result<(), Error> {
    let key_ptr = input.first().and_then(Val::i64).unwrap_or(-1);
    let key_len = input.get(1).and_then(Val::i64).unwrap_or(-1);
    let iv_ptr = input.get(2).and_then(Val::i64).unwrap_or(-1);
    let buf_ptr = input.get(3).and_then(Val::i64).unwrap_or(-1);
    let buf_len = input.get(4).and_then(Val::i64).unwrap_or(-1);

    output[0] = Val::I64(aes_cbc_decrypt_raw(
        current, key_ptr, key_len, iv_ptr, buf_ptr, buf_len,
    ));
    Ok(())
}

fn scryer_crc32(
    current: &mut CurrentPlugin,
    input: &[Val],
    output: &mut [Val],
    _state: UserData<()>,
) -> Result<(), Error> {
    let seed = input.first().and_then(Val::i64).unwrap_or(0) as u32;
    let buf_ptr = input.get(1).and_then(Val::i64).unwrap_or(-1);
    let buf_len = input.get(2).and_then(Val::i64).unwrap_or(-1);

    output[0] = Val::I64(crc32_raw(current, seed, buf_ptr, buf_len));
    Ok(())
}

fn aes_cbc_decrypt_raw(
    current: &mut CurrentPlugin,
    key_ptr: i64,
    key_len: i64,
    iv_ptr: i64,
    buf_ptr: i64,
    buf_len: i64,
) -> i64 {
    if key_len != AES_128_KEY_LEN as i64 && key_len != AES_256_KEY_LEN as i64 {
        return AES_STATUS_BAD_KEY_LEN;
    }
    if buf_len < 0 || buf_len % AES_BLOCK_LEN as i64 != 0 {
        return AES_STATUS_BAD_BLOCK_LEN;
    }

    let Ok(key_handle) = checked_memory_handle(current, key_ptr, key_len as u64) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };
    let Ok(iv_handle) = checked_memory_handle(current, iv_ptr, AES_BLOCK_LEN as u64) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };
    let Ok(buf_handle) = checked_memory_handle(current, buf_ptr, buf_len as u64) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };

    let Ok(key) = current.memory_bytes(key_handle) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };
    let key = key.to_vec();

    let Ok(iv) = current.memory_bytes(iv_handle) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };
    let mut iv_bytes = [0u8; AES_BLOCK_LEN];
    iv_bytes.copy_from_slice(iv);

    if buf_len == 0 {
        return AES_STATUS_OK;
    }

    let Ok(buf) = current.memory_bytes_mut(buf_handle) else {
        return AES_STATUS_OUT_OF_BOUNDS;
    };

    match aes_cbc_decrypt_in_place(&key, &iv_bytes, buf) {
        Ok(()) => AES_STATUS_OK,
        Err(AesDecryptError::BadKeyLen) => AES_STATUS_BAD_KEY_LEN,
        Err(AesDecryptError::BadBlockLen) => AES_STATUS_BAD_BLOCK_LEN,
    }
}

fn crc32_raw(current: &mut CurrentPlugin, seed: u32, buf_ptr: i64, buf_len: i64) -> i64 {
    if buf_len < 0 {
        return CRC_STATUS_OUT_OF_BOUNDS;
    }
    let Ok(buf_handle) = checked_memory_handle(current, buf_ptr, buf_len as u64) else {
        return CRC_STATUS_OUT_OF_BOUNDS;
    };
    let Ok(buf) = current.memory_bytes(buf_handle) else {
        return CRC_STATUS_OUT_OF_BOUNDS;
    };
    crc32(seed, buf) as i64
}

fn crc32(seed: u32, buf: &[u8]) -> u32 {
    // `new_with_initial` resumes from a finalized IEEE CRC value, which is the
    // guest ABI contract for streaming archive verification.
    let mut hasher = crc32fast::Hasher::new_with_initial(seed);
    hasher.update(buf);
    hasher.finalize()
}

fn checked_memory_handle(
    _current: &mut CurrentPlugin,
    ptr: i64,
    len: u64,
) -> Result<MemoryHandle, ()> {
    if ptr < 0 {
        return Err(());
    }
    let ptr = ptr as u64;
    ptr.checked_add(len).ok_or(())?;
    Ok(unsafe { MemoryHandle::new(ptr, len) })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AesDecryptError {
    BadKeyLen,
    BadBlockLen,
}

fn aes_cbc_decrypt_in_place(
    key: &[u8],
    iv: &[u8; AES_BLOCK_LEN],
    buf: &mut [u8],
) -> Result<(), AesDecryptError> {
    if !matches!(key.len(), AES_128_KEY_LEN | AES_256_KEY_LEN) {
        return Err(AesDecryptError::BadKeyLen);
    }
    if !buf.len().is_multiple_of(AES_BLOCK_LEN) {
        return Err(AesDecryptError::BadBlockLen);
    }
    if buf.is_empty() {
        return Ok(());
    }

    let mut aes_key = MaybeUninit::<aws_lc_sys::AES_KEY>::uninit();
    let bits = (key.len() * 8) as u32;
    let set_key_result =
        unsafe { aws_lc_sys::AES_set_decrypt_key(key.as_ptr(), bits, aes_key.as_mut_ptr()) };
    if set_key_result != 0 {
        return Err(AesDecryptError::BadKeyLen);
    }
    let aes_key = unsafe { aes_key.assume_init() };
    let mut iv = *iv;
    unsafe {
        aws_lc_sys::AES_cbc_encrypt(
            buf.as_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            &aes_key,
            iv.as_mut_ptr(),
            aws_lc_sys::AES_DECRYPT,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_cbc_decrypts_nist_aes128_vector() {
        let key = hex_bytes("2b7e151628aed2a6abf7158809cf4f3c");
        let iv: [u8; AES_BLOCK_LEN] = hex_bytes("000102030405060708090a0b0c0d0e0f")
            .try_into()
            .unwrap();
        let mut buf = hex_bytes(
            "7649abac8119b246cee98e9b12e9197d\
             5086cb9b507219ee95db113a917678b2",
        );
        let expected = hex_bytes(
            "6bc1bee22e409f96e93d7e117393172a\
             ae2d8a571e03ac9c9eb76fac45af8e51",
        );

        aes_cbc_decrypt_in_place(&key, &iv, &mut buf).unwrap();

        assert_eq!(buf, expected);
    }

    #[test]
    fn aes_cbc_decrypt_accepts_empty_buffer() {
        let key = [0u8; AES_128_KEY_LEN];
        let iv = [0u8; AES_BLOCK_LEN];
        let mut buf = [];

        aes_cbc_decrypt_in_place(&key, &iv, &mut buf).unwrap();
    }

    #[test]
    fn aes_cbc_decrypt_rejects_invalid_lengths() {
        let iv = [0u8; AES_BLOCK_LEN];
        assert_eq!(
            aes_cbc_decrypt_in_place(&[0u8; 15], &iv, &mut [0u8; AES_BLOCK_LEN]),
            Err(AesDecryptError::BadKeyLen)
        );
        assert_eq!(
            aes_cbc_decrypt_in_place(&[0u8; AES_128_KEY_LEN], &iv, &mut [0u8; 15]),
            Err(AesDecryptError::BadBlockLen)
        );
    }

    #[test]
    fn crc32_matches_ieee_check_value() {
        assert_eq!(crc32(0, b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn crc32_chains_from_running_seed() {
        let first = crc32(0, b"archive ");
        let chained = crc32(first, b"payload");
        let combined = crc32(0, b"archive payload");

        assert_eq!(chained, combined);
    }

    fn hex_bytes(input: &str) -> Vec<u8> {
        let compact = input
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert_eq!(compact.len() % 2, 0);
        (0..compact.len())
            .step_by(2)
            .map(|idx| u8::from_str_radix(&compact[idx..idx + 2], 16).unwrap())
            .collect()
    }
}
