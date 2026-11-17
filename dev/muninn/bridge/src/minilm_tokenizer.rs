//! Bounded pinned tokenization; not sentence inference or numerical acceptance.

use tokenizers::{PostProcessor, Tokenizer};
#[path = "minilm_tokenizer_profile.rs"]
mod profile;

const MAX_BATCH: usize = 32;
const MAX_INPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum TokenizerError {
    Cache(crate::artifact_fs::staging::model::CacheError),
    Configuration,
    InputLimit,
    Tokenization,
    Output,
}

pub struct MiniLmTokenizer {
    tokenizer: Tokenizer,
}

/// No Debug or mutable getters: token arrays can reveal user input.
pub struct TokenBatch {
    input_ids: Vec<Vec<u32>>,
    token_type_ids: Vec<Vec<u32>>,
    attention_mask: Vec<Vec<u32>>,
}

impl TokenBatch {
    pub fn input_ids(&self) -> &[Vec<u32>] {
        &self.input_ids
    }
    pub fn token_type_ids(&self) -> &[Vec<u32>] {
        &self.token_type_ids
    }
    pub fn attention_mask(&self) -> &[Vec<u32>] {
        &self.attention_mask
    }

    fn checked(self, input_count: usize) -> Result<Self, TokenizerError> {
        if input_count == 0
            || input_count > MAX_BATCH
            || self.input_ids.len() != input_count
            || self.token_type_ids.len() != input_count
            || self.attention_mask.len() != input_count
        {
            return Err(TokenizerError::Output);
        }
        let width = self.input_ids[0].len();
        if !(2..=256).contains(&width) {
            return Err(TokenizerError::Output);
        }
        let mut longest = 0;
        for index in 0..input_count {
            let ids = &self.input_ids[index];
            let types = &self.token_type_ids[index];
            let mask = &self.attention_mask[index];
            if ids.len() != width
                || types.len() != width
                || mask.len() != width
                || ids.iter().any(|id| *id >= 30522)
                || types.iter().any(|id| *id != 0)
            {
                return Err(TokenizerError::Output);
            }
            let real = mask.iter().take_while(|value| **value == 1).count();
            if real < 2
                || mask[real..].iter().any(|value| *value != 0)
                || ids[0] != 101
                || ids[real - 1] != 102
                || ids[real..].iter().any(|id| *id != 0)
            {
                return Err(TokenizerError::Output);
            }
            longest = longest.max(real);
        }
        if longest != width {
            return Err(TokenizerError::Output);
        }
        Ok(self)
    }
}

impl MiniLmTokenizer {
    pub fn from_cache(
        cache: &crate::artifact_fs::staging::model::VerifiedArtifactCache,
    ) -> Result<Self, TokenizerError> {
        let tokenizer = cache
            .read_artifact("tokenizer.json")
            .map_err(TokenizerError::Cache)?;
        let sentence = cache
            .read_artifact("sentence_bert_config.json")
            .map_err(TokenizerError::Cache)?;
        let config = cache
            .read_artifact("tokenizer_config.json")
            .map_err(TokenizerError::Cache)?;
        let specials = cache
            .read_artifact("special_tokens_map.json")
            .map_err(TokenizerError::Cache)?;
        profile::validate(
            tokenizer.as_bytes(),
            sentence.as_bytes(),
            config.as_bytes(),
            specials.as_bytes(),
        )?;
        let tokenizer = Tokenizer::from_bytes(tokenizer.as_bytes())
            .map_err(|_| TokenizerError::Configuration)?;
        if tokenizer.get_vocab_size(true) != 30522 {
            return Err(TokenizerError::Configuration);
        }
        Self::configure(tokenizer)
    }

    fn configure(mut tokenizer: Tokenizer) -> Result<Self, TokenizerError> {
        if tokenizer
            .get_post_processor()
            .map(|processor| processor.added_tokens(false))
            != Some(2)
        {
            return Err(TokenizerError::Configuration);
        }
        for (token, id) in [
            ("[PAD]", 0),
            ("[UNK]", 100),
            ("[CLS]", 101),
            ("[SEP]", 102),
            ("[MASK]", 103),
        ] {
            if tokenizer.token_to_id(token) != Some(id) {
                return Err(TokenizerError::Configuration);
            }
        }
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::BatchLongest,
            direction: tokenizers::PaddingDirection::Right,
            pad_to_multiple_of: None,
            pad_id: 0,
            pad_type_id: 0,
            pad_token: "[PAD]".to_owned(),
        }));
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                direction: tokenizers::TruncationDirection::Right,
                max_length: 256,
                strategy: tokenizers::TruncationStrategy::LongestFirst,
                stride: 0,
            }))
            .map_err(|_| TokenizerError::Configuration)?;
        Ok(Self { tokenizer })
    }

    pub fn encode(&self, texts: &[&str]) -> Result<TokenBatch, TokenizerError> {
        if texts.is_empty() || texts.len() > MAX_BATCH {
            return Err(TokenizerError::InputLimit);
        }
        let bytes = texts
            .iter()
            .try_fold(0_usize, |sum, text| sum.checked_add(text.len()))
            .ok_or(TokenizerError::InputLimit)?;
        if bytes > MAX_INPUT_BYTES {
            return Err(TokenizerError::InputLimit);
        }
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|_| TokenizerError::Tokenization)?;
        TokenBatch {
            input_ids: encodings.iter().map(|e| e.get_ids().to_vec()).collect(),
            token_type_ids: encodings
                .iter()
                .map(|e| e.get_type_ids().to_vec())
                .collect(),
            attention_mask: encodings
                .iter()
                .map(|e| e.get_attention_mask().to_vec())
                .collect(),
        }
        .checked(texts.len())
    }
}

#[cfg(test)]
#[path = "minilm_tokenizer_tests.rs"]
mod tests;
