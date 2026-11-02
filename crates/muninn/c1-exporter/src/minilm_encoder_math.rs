use super::EncoderError;
use candle_core::{DType, Tensor};

pub(super) fn runtime<T>(result: candle_core::Result<T>) -> Result<T, EncoderError> {
    result.map_err(|_| EncoderError::Runtime)
}

fn float_execution(tensor: &Tensor) -> Result<(), EncoderError> {
    if tensor.dtype() != DType::F32 || !tensor.device().is_cpu() || tensor.track_op() {
        return Err(EncoderError::Precision);
    }
    Ok(())
}

fn finite(tensor: &Tensor) -> Result<(), EncoderError> {
    if runtime(runtime(tensor.flatten_all())?.to_vec1::<f32>())?
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(EncoderError::Values);
    }
    Ok(())
}

pub(super) fn masked_mean(hidden: &Tensor, mask: &Tensor) -> Result<Tensor, EncoderError> {
    let (batch, sequence, dimension) = hidden.dims3().map_err(|_| EncoderError::Shape)?;
    if !(1..=32).contains(&batch)
        || !(2..=256).contains(&sequence)
        || dimension != 384
        || mask.dims() != [batch, sequence]
    {
        return Err(EncoderError::Shape);
    }
    float_execution(hidden)?;
    if mask.dtype() != DType::U32 || !mask.device().is_cpu() || mask.track_op() {
        return Err(EncoderError::Precision);
    }
    finite(hidden)?;
    let rows = runtime(mask.to_vec2::<u32>())?;
    if rows
        .iter()
        .any(|row| row.iter().any(|value| *value > 1) || row.iter().all(|value| *value == 0))
    {
        return Err(EncoderError::Values);
    }
    let expanded = runtime(runtime(mask.to_dtype(DType::F32))?.unsqueeze(2))?;
    let numerator = runtime(runtime(hidden.broadcast_mul(&expanded))?.sum(1))?;
    let denominator = runtime(runtime(expanded.sum(1))?.clamp(1e-9, f64::from(f32::MAX)))?;
    let pooled = runtime(numerator.broadcast_div(&denominator))?;
    finite(&pooled)?;
    Ok(pooled)
}

pub(super) fn normalize(pooled: &Tensor) -> Result<Tensor, EncoderError> {
    let (batch, dimension) = pooled.dims2().map_err(|_| EncoderError::Shape)?;
    if !(1..=32).contains(&batch) || dimension != 384 {
        return Err(EncoderError::Shape);
    }
    float_execution(pooled)?;
    finite(pooled)?;
    let norms = runtime(runtime(runtime(pooled.sqr())?.sum_keepdim(1))?.sqrt())?;
    let values = runtime(norms.to_vec2::<f32>())?;
    if values
        .iter()
        .flatten()
        .any(|value| !value.is_finite() || *value <= 1e-12_f32)
    {
        return Err(EncoderError::Values);
    }
    let denominator = runtime(norms.clamp(1e-12, f64::from(f32::MAX)))?;
    let normalized = runtime(pooled.broadcast_div(&denominator))?;
    finite(&normalized)?;
    Ok(normalized)
}
