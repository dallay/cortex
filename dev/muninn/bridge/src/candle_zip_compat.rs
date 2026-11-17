//! Benign archive-I/O compatibility checks; no extraction or model inference.

use candle_core::{DType, Device, Tensor};

#[test]
fn explicit_f32_cpu_profile_preserves_matrix_product_bits() {
    let left = Tensor::new(&[[1_f32, 2_f32], [3_f32, 4_f32]], &Device::Cpu).expect("F32 left");
    let right = Tensor::new(&[[5_f32, 6_f32], [7_f32, 8_f32]], &Device::Cpu).expect("F32 right");
    let product = left.matmul(&right).expect("supported CPU F32 GEMM");
    assert_eq!(product.dtype(), DType::F32);
    assert_eq!(product.dims(), &[2, 2]);
    let bits: Vec<_> = product
        .to_vec2::<f32>()
        .expect("F32 output")
        .into_iter()
        .flatten()
        .map(f32::to_bits)
        .collect();
    assert_eq!(bits, [19_f32, 22_f32, 43_f32, 50_f32].map(f32::to_bits));
}

#[test]
fn npz_round_trip_preserves_cpu_f32_names_shapes_and_value_bits() {
    let fixture = tempfile::tempdir().expect("temporary tensor archive");
    let path = fixture.path().join("tensor.npz");
    let matrix = Tensor::new(&[[1_f32, 2_f32], [3_f32, 4_f32]], &Device::Cpu).expect("CPU tensor");
    Tensor::write_npz(&[("matrix", &matrix), ("café", &matrix)], &path).expect("write benign NPZ");
    let values = Tensor::read_npz(&path).expect("read all arrays");
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].0, "matrix");
    assert_eq!(values[1].0, "café");
    for (_, tensor) in values {
        assert_eq!(tensor.dtype(), DType::F32);
        assert_eq!(tensor.dims(), &[2, 2]);
        let bits: Vec<_> = tensor
            .to_vec2::<f32>()
            .expect("F32 data")
            .into_iter()
            .flatten()
            .map(f32::to_bits)
            .collect();
        assert_eq!(bits, [1_f32, 2_f32, 3_f32, 4_f32].map(f32::to_bits));
    }
    let archive = candle_core::npy::NpzTensors::new(&path).expect("lazy NPZ index");
    let tensor = archive
        .get("matrix")
        .expect("lazy read")
        .expect("known array");
    assert_eq!(tensor.dtype(), DType::F32);
    assert_eq!(tensor.dims(), &[2, 2]);
    let lazy_bits: Vec<_> = tensor
        .to_vec2::<f32>()
        .expect("lazy F32 data")
        .into_iter()
        .flatten()
        .map(f32::to_bits)
        .collect();
    assert_eq!(lazy_bits, [1_f32, 2_f32, 3_f32, 4_f32].map(f32::to_bits));
    assert!(archive.get("missing").expect("missing array").is_none());
    assert!(Tensor::read_npz_by_name(&path, &["missing"]).is_err());
}

#[test]
fn pickle_archive_name_iteration_handles_a_non_pickle_npz_without_execution() {
    let fixture = tempfile::tempdir().expect("temporary archive");
    let path = fixture.path().join("not-a-model.npz");
    let tensor = Tensor::new(&[1_f32], &Device::Cpu).expect("CPU tensor");
    Tensor::write_npz(&[("plain-array", &tensor)], &path).expect("write benign NPZ");
    let infos = candle_core::pickle::read_pth_tensor_info(&path, false, None)
        .expect("fallible archive names");
    assert!(infos.is_empty());
}

#[test]
fn non_utf8_archive_names_keep_zip9_cp437_compatibility() {
    let fixture = tempfile::tempdir().expect("temporary malformed archive");
    let path = fixture.path().join("invalid-name.npz");
    let tensor = Tensor::new(&[1_f32], &Device::Cpu).expect("benign CPU tensor");
    Tensor::write_npz(&[("array", &tensor)], &path).expect("valid starting fixture");
    let mut bytes = std::fs::read(&path).expect("archive bytes");
    // Mutate only this tiny fixture's local/central filename and UTF-8 flag.
    // Tensor payload/CRC are unchanged. No archive member is extracted.
    for (signature, flags_offset, name_offset) in [(b"PK\x03\x04", 6, 30), (b"PK\x01\x02", 8, 46)] {
        let offset = bytes
            .windows(4)
            .position(|window| window == signature)
            .expect("fixture header");
        let flags = u16::from_le_bytes([
            bytes[offset + flags_offset],
            bytes[offset + flags_offset + 1],
        ]) | (1 << 11);
        bytes[offset + flags_offset..offset + flags_offset + 2]
            .copy_from_slice(&flags.to_le_bytes());
        bytes[offset + name_offset] = 0xff;
    }
    std::fs::write(&path, bytes).expect("malformed fixture only");
    let values = Tensor::read_npz(&path).expect("ZIP 9 decodes non-UTF8 using CP437");
    assert_eq!(values[0].0, "\u{00a0}rray");
    let archive = candle_core::npy::NpzTensors::new(&path).expect("decoded name index");
    assert!(archive.get("\u{00a0}rray").expect("CP437 name").is_some());
    assert!(
        candle_core::pickle::read_pth_tensor_info(&path, false, None)
            .expect("decoded names")
            .is_empty()
    );
}

#[test]
fn truncated_archives_fail_closed_in_eager_lazy_and_pickle_readers() {
    let fixture = tempfile::tempdir().expect("temporary corrupt archive");
    let path = fixture.path().join("truncated.npz");
    let tensor = Tensor::new(&[1_f32], &Device::Cpu).expect("benign CPU tensor");
    Tensor::write_npz(&[("array", &tensor)], &path).expect("valid fixture");
    let bytes = std::fs::read(&path).expect("fixture bytes");
    std::fs::write(&path, &bytes[..bytes.len() / 2]).expect("truncate temporary fixture");
    assert!(Tensor::read_npz(&path).is_err());
    assert!(candle_core::npy::NpzTensors::new(&path).is_err());
    assert!(candle_core::pickle::read_pth_tensor_info(&path, false, None).is_err());
}
