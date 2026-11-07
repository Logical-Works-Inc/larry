//! The local engine behind `larry-rerank`: the pinned Qwen3-Reranker-0.6B on Candle, its prompts
//! encoded by the Hugging Face tokenizer, both through the lgwks_deps storefront (`ml-candle`,
//! `ml-tokenizers`). It reads the checkpoint from the Hugging Face cache only and never calls the
//! hub, as the Python tool's `local_files_only=True` did. Each candidate's score is the
//! probability of "yes" against "no" at the last position, exactly the Python
//! `log_softmax(...)[:, 1].exp()`. Candidates run one sequence at a time: the Python tool padded
//! a batch on the left under an attention mask, which leaves every row's logits equal to its
//! unpadded run, so batching changes only throughput. Source: the Qwen3-Reranker model card.

use std::path::{Path, PathBuf};

use lgwks_deps::candle_core::{DType, Device, Tensor};
use lgwks_deps::candle_nn::VarBuilder;
use lgwks_deps::candle_transformers::models::qwen3::{Config, ModelForCausalLM};
use lgwks_deps::tokenizers::Tokenizer;
use lgwks_std::json;

use super::{Candidate, Cause, Deadline, MODEL_ID, MODEL_REVISION, RerankError, Scorer};

/// Most tokens one judged prompt may hold, prefix and suffix included.
const MAX_LENGTH: usize = 8192;
/// The task instruction every prompt carries.
const INSTRUCTION: &str =
    "Given a software-engineering query, retrieve relevant code and documentation passages.";
/// The system turn and the opening of the user turn.
const PREFIX: &str = "<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be \"yes\" or \"no\".<|im_end|>\n<|im_start|>user\n";
/// The close of the user turn and an empty thinking block before the answer.
const SUFFIX: &str = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";

/// The loaded model and its prompt pieces.
struct Loaded {
    /// The causal language model.
    model: ModelForCausalLM,
    /// The checkpoint's tokenizer.
    tokenizer: Tokenizer,
    /// The prefix's token ids.
    prefix: Vec<u32>,
    /// The suffix's token ids.
    suffix: Vec<u32>,
    /// The token id of "yes".
    yes: usize,
    /// The token id of "no".
    no: usize,
}

/// The engine, loading the checkpoint on first use so the request's time limit covers loading.
pub(crate) struct Qwen {
    /// The snapshot directory holding `config.json`, `tokenizer.json` and `model.safetensors`.
    snapshot: PathBuf,
    /// The model once loaded.
    loaded: Option<Loaded>,
}

/// An engine failure, with what was being done and its cause kept.
fn engine<E: Into<Cause>>(doing: &'static str) -> impl FnOnce(E) -> RerankError {
    move |cause| RerankError::EngineFault {
        doing,
        cause: cause.into(),
    }
}

impl Qwen {
    /// The engine over the pinned snapshot in the Hugging Face cache: `HF_HUB_CACHE`, else
    /// `HF_HOME/hub`, else `~/.cache/huggingface/hub`, as the hub's own clients resolve it.
    pub(crate) fn from_cache() -> Result<Self, RerankError> {
        let hub = match std::env::var_os("HF_HUB_CACHE") {
            Some(hub) => PathBuf::from(hub),
            None => match std::env::var_os("HF_HOME") {
                Some(home) => PathBuf::from(home).join("hub"),
                None => {
                    match std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
                        Some(home) => PathBuf::from(home)
                            .join(".cache")
                            .join("huggingface")
                            .join("hub"),
                        None => {
                            return Err(RerankError::Engine(String::from(
                                "no HF_HUB_CACHE, HF_HOME or home directory to find the model in",
                            )));
                        }
                    }
                }
            },
        };
        let repository = format!("models--{}", MODEL_ID.replace('/', "--"));
        Ok(Self {
            snapshot: hub.join(repository).join("snapshots").join(MODEL_REVISION),
            loaded: None,
        })
    }

    /// Loads the tokenizer, the configuration and the weights from the snapshot.
    fn load(snapshot: &Path) -> Result<Loaded, RerankError> {
        let tokenizer = Tokenizer::from_file(snapshot.join("tokenizer.json"))
            .map_err(engine("reading tokenizer.json"))?;
        let text = std::fs::read_to_string(snapshot.join("config.json"))
            .map_err(engine("reading config.json"))?;
        let config: Config = json::from_str(&text).map_err(engine("parsing config.json"))?;
        let device = Device::Cpu;
        let tensors =
            lgwks_deps::candle_core::safetensors::load(snapshot.join("model.safetensors"), &device)
                .map_err(engine("reading model.safetensors"))?;
        let weights = VarBuilder::from_tensors(tensors, DType::F32, &device);
        let model =
            ModelForCausalLM::new(&config, weights).map_err(engine("building the model"))?;
        let id = |token: &str| -> Result<usize, RerankError> {
            let id = tokenizer.token_to_id(token).ok_or_else(|| {
                RerankError::Engine(format!("the tokenizer has no {token:?} token"))
            })?;
            usize::try_from(id).map_err(engine("widening a token id"))
        };
        let (yes, no) = (id("yes")?, id("no")?);
        let encode = |text: &str| {
            tokenizer
                .encode(text, false)
                .map(|encoding| encoding.get_ids().to_vec())
                .map_err(engine("encoding the prompt frame"))
        };
        let (prefix, suffix) = (encode(PREFIX)?, encode(SUFFIX)?);
        Ok(Loaded {
            model,
            tokenizer,
            prefix,
            suffix,
            yes,
            no,
        })
    }
}

impl Loaded {
    /// The probability of "yes" for one prompt body, truncated to fit [`MAX_LENGTH`].
    fn judge(&mut self, body: &str) -> Result<f64, RerankError> {
        let room = MAX_LENGTH
            .saturating_sub(self.prefix.len())
            .saturating_sub(self.suffix.len());
        let encoding = self
            .tokenizer
            .encode(body, false)
            .map_err(engine("encoding a candidate"))?;
        let body_ids = encoding.get_ids();
        let kept = body_ids.get(..room.min(body_ids.len())).ok_or_else(|| {
            RerankError::Engine(String::from("truncation past the encoded candidate"))
        })?;
        let ids: Vec<u32> = self
            .prefix
            .iter()
            .chain(kept)
            .chain(self.suffix.iter())
            .copied()
            .collect();
        self.model.clear_kv_cache();
        let input = Tensor::new(ids.as_slice(), &Device::Cpu)
            .and_then(|tensor| tensor.unsqueeze(0))
            .map_err(engine("building the input"))?;
        let logits = self
            .model
            .forward(&input, 0)
            .and_then(|logits| logits.flatten_all())
            .and_then(|logits| logits.to_dtype(DType::F32))
            .map_err(engine("running the model"))?;
        let logit = |id: usize| {
            logits
                .get(id)
                .and_then(|value| value.to_scalar::<f32>())
                .map(f64::from)
                .map_err(engine("reading a logit"))
        };
        let (yes, no) = (logit(self.yes)?, logit(self.no)?);
        self.model.clear_kv_cache();
        Ok(1.0 / (1.0 + (no - yes).exp()))
    }
}

impl Scorer for Qwen {
    fn score(
        &mut self,
        query: &str,
        rows: &[Candidate],
        _batch: usize,
        deadline: Deadline,
    ) -> Result<Vec<f64>, RerankError> {
        let loaded = match self.loaded {
            Some(ref mut loaded) => loaded,
            None => self.loaded.insert(Self::load(&self.snapshot)?),
        };
        let mut scores = Vec::with_capacity(rows.len());
        for row in rows {
            deadline.check()?;
            let body = format!(
                "<Instruct>: {INSTRUCTION}\n<Query>: {query}\n<Document>: {}",
                row.text()
            );
            scores.push(loaded.judge(&body)?);
        }
        deadline.check()?;
        Ok(scores)
    }
}
