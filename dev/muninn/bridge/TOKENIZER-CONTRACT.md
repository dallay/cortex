# MiniLM tokenizer contract

`MiniLmTokenizer::from_cache` accepts only the verified artifact cache. It reads
fresh tokenizer, sentence-profile, tokenizer-config and special-token-map buffers.
It preserves the pinned WordPiece/BertNormalizer/TemplateProcessing pipeline and
checks its profile and vocabulary special IDs before changing runtime limits.

## Runtime behavior

- Single text inputs only; no prompt, prefix, text pair or stored-content mutation.
- No extra lowercase phase. The pinned BertNormalizer already lowercases; the
  sentence profile's separate `do_lower_case` flag is false.
- Replace the embedded 128-token truncation with right `longest_first`, stride
  zero, maximum 256 **including** `[CLS]` and `[SEP]`.
- Replace fixed-128 padding with right padding to the longest sequence in the
  current batch. Padding ID/type ID are zero; no padding multiple is requested.
- Apply the pinned special-token postprocessor once. Literal special-token text
  remains input content; a literal `[PAD]` with mask one is not batch padding.

`encode` rejects empty batches, more than 32 texts or more than 1 MiB total UTF-8
input before backend encoding. Individual empty strings are valid. These finite
prototype limits do not establish matching upstream service limits, intermediate
allocation limits, execution deadlines or global admission quotas.

The returned batch has readonly getters and no `Debug` implementation because
token arrays can reveal user text. Output count must equal input count. IDs,
types and masks have equal rectangular widths, and that width must equal the
longest real-token count, between 2 and 256. Masks are contiguous ones followed
by padding zeros; types are zero. IDs stay inside the model vocabulary, with CLS
first, SEP last among real tokens and zero IDs in padding. Errors contain static
variants, not input text or backend error strings.

## Reference fixtures

`dev/muninn/minilm_tokenizer_reference.py` generates public synthetic fixtures
using the pinned Python fast tokenizer wrapper and its pinned backend. It does
not import or run a SentenceTransformer model. The separate tooling environment
contains no torch model runtime; model Hub access is disabled during generation.

The generator verifies the selected local files before and after generation,
checks the exact installed dependency versions and records artifact, requirement,
generator and upstream-lock hashes. The Python wrapper reopens checked paths;
reference integrity still depends on the trusted bundle remaining stable during
execution. These checks are snapshots, not an immutable file lease or a substitute
for the Rust production verified-buffer boundary.

The selected wheel hashes in `requirements-minilm-tokenizer.txt` reproduce the
macOS ARM64/Python 3.13 reference environment; they are not a portable wheel set
for every platform. The pip report records their provenance. `pip check` is a
dependency-consistency check, not a Python vulnerability audit.

Golden JSON contains complete inputs, IDs, types and masks for ten batches and
nineteen texts. It includes Unicode, control characters, literal special tokens,
truncation boundaries and mixed-length padding. Its full generated size counts
toward delivery review; generated fixtures are not a PR-size exception.

```sh
HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 USE_TORCH=0 \
  TOKENIZERS_PARALLELISM=false <reference-python> -B \
  dev/muninn/minilm_tokenizer_reference.py /physical/pinned-bundle

MUNINN_MINILM_ARTIFACT_ROOT=/physical/private-root \
  cargo test -p muninn-bridge-spike --lib --locked --offline \
  minilm_tokenizer::tests::actual_cache_matches -- --ignored --nocapture
```

The Rust test binds fixtures to their source/settings/hashes and compares all
three arrays exactly, plus single-versus-batched nonpadding consistency. It must
be explicitly executed; its default ignored status is not a pass. Routine unit
tests use a small labeled synthetic tokenizer rather than copying the full model
vocabulary or changing private cache permissions.

Exact fixture equality is tokenizer-only evidence across the recorded versions
and cases. It does not establish sentence embeddings, full SentenceTransformer
runtime equivalence, evaluation-forward behavior, masked pooling, L2, model batch
invariance, C0/C1, TCK or functional parity.
