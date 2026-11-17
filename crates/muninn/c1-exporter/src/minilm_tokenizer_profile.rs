//! Extra profile checks after manifest verification; no rewriting of normalization.

use super::TokenizerError;
use serde_json::{json, Value};

fn parse(bytes: &[u8]) -> Result<Value, TokenizerError> {
    if bytes.len() > 1024 * 1024 {
        return Err(TokenizerError::Configuration);
    }
    serde_json::from_slice(bytes).map_err(|_| TokenizerError::Configuration)
}

pub(super) fn validate(
    tokenizer: &[u8],
    sentence: &[u8],
    config: &[u8],
    specials: &[u8],
) -> Result<(), TokenizerError> {
    let t = parse(tokenizer)?;
    let sentence = parse(sentence)?;
    let config = parse(config)?;
    let specials = parse(specials)?;
    if sentence != json!({"do_lower_case":false,"max_seq_length":256})
        || t["model"]["type"] != "WordPiece"
        || t["model"]["unk_token"] != "[UNK]"
        || t["model"]["continuing_subword_prefix"] != "##"
        || t["model"]["max_input_chars_per_word"] != 100
        || t["model"]["vocab"].as_object().map(serde_json::Map::len) != Some(30522)
        || t["normalizer"]
            != json!({"type":"BertNormalizer","clean_text":true,"handle_chinese_chars":true,"strip_accents":null,"lowercase":true})
        || t["pre_tokenizer"] != json!({"type":"BertPreTokenizer"})
        || t["decoder"] != json!({"type":"WordPiece","prefix":"##","cleanup":true})
        || t["truncation"] != json!({"max_length":128,"strategy":"LongestFirst","stride":0})
        || t["padding"]
            != json!({"strategy":{"Fixed":128},"direction":"Right","pad_to_multiple_of":null,"pad_id":0,"pad_type_id":0,"pad_token":"[PAD]"})
        || t["post_processor"]["type"] != "TemplateProcessing"
        || t["post_processor"]["single"]
            != json!([
                {"SpecialToken":{"id":"[CLS]","type_id":0}},
                {"Sequence":{"id":"A","type_id":0}},
                {"SpecialToken":{"id":"[SEP]","type_id":0}}
            ])
        || specials
            != json!({"unk_token":"[UNK]","sep_token":"[SEP]","pad_token":"[PAD]","cls_token":"[CLS]","mask_token":"[MASK]"})
        || config["tokenizer_class"] != "BertTokenizer"
        || config["model_max_length"] != 512
        || config["do_lower_case"] != true
        || config["do_basic_tokenize"] != true
        || config["tokenize_chinese_chars"] != true
        || !config["strip_accents"].is_null()
        || !config["never_split"].is_null()
    {
        return Err(TokenizerError::Configuration);
    }
    for name in [
        "unk_token",
        "sep_token",
        "pad_token",
        "cls_token",
        "mask_token",
    ] {
        if config[name] != specials[name] {
            return Err(TokenizerError::Configuration);
        }
    }
    Ok(())
}
