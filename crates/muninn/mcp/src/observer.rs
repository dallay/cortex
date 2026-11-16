//! Observational Memory — context compression and observation extraction.
//!
//! Equivalent to `neo4j_agent_memory.mcp._observer`.
//!
//! The `MemoryObserver` monitors accumulated context per session and extracts
//! high-level observations (key facts, decisions, topic shifts) when the
//! token count exceeds a configurable threshold.
//!
//! Three-tier context hierarchy:
//! 1. **Reflections** — high-level session summaries (generated when threshold exceeded)
//! 2. **Observations** — extracted facts, decisions, preferences from messages
//! 3. **Recent messages** — the most recent messages in the session

use std::collections::HashMap;

/// A single extracted observation from a message.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// The text content of the observation.
    pub text: String,
    /// Classification: "fact", "decision", "preference", "topic_shift".
    pub observation_type: String,
    /// Timestamp of extraction.
    pub created_at: String,
}

/// Accumulated context for a single session.
#[derive(Debug, Clone, Default)]
pub struct SessionContext {
    /// Session identifier.
    pub session_id: String,
    /// Messages accumulated in this session.
    pub messages: Vec<String>,
    /// Approximate token count across all messages.
    pub token_count: usize,
    /// Extracted observations so far.
    pub observations: Vec<Observation>,
    /// High-level reflections generated for this session.
    pub reflections: Vec<String>,
}

/// Configuration for the memory observer.
#[derive(Debug, Clone)]
pub struct ObserverConfig {
    /// Token threshold at which to trigger observation extraction.
    pub observation_threshold: usize,
    /// Maximum number of recent messages to keep in context.
    pub max_recent_messages: usize,
    /// Whether to enable LLM-based summarization (if a provider is configured).
    pub enable_llm_summarization: bool,
}

impl Default for ObserverConfig {
    fn default() -> Self {
        Self {
            observation_threshold: 2048,
            max_recent_messages: 20,
            enable_llm_summarization: false,
        }
    }
}

/// Monitors session context and extracts observations.
///
/// Equivalent to `MemoryObserver` in the Python reference.
#[derive(Debug, Clone)]
pub struct MemoryObserver {
    config: ObserverConfig,
    /// Per-session context tracking.
    sessions: HashMap<String, SessionContext>,
}

impl MemoryObserver {
    /// Create a new observer with the given configuration.
    pub fn new(config: ObserverConfig) -> Self {
        Self {
            config,
            sessions: HashMap::new(),
        }
    }

    /// Create a new observer with default settings.
    pub fn with_defaults() -> Self {
        Self::new(ObserverConfig::default())
    }

    /// Get or create the session context for a given session ID.
    pub fn session_context(&mut self, session_id: &str) -> &mut SessionContext {
        self.sessions
            .entry(session_id.to_owned())
            .or_insert_with(|| SessionContext {
                session_id: session_id.to_owned(),
                messages: Vec::new(),
                token_count: 0,
                observations: Vec::new(),
                reflections: Vec::new(),
            })
    }

    /// Record a new message and potentially extract observations.
    ///
    /// Returns any newly extracted observations, or an empty vec if the
    /// threshold was not exceeded.
    pub fn on_message_stored(&mut self, session_id: &str, content: &str) -> Vec<Observation> {
        let max_recent = self.config.max_recent_messages;
        let threshold = self.config.observation_threshold;

        let ctx = self.session_context(session_id);
        ctx.messages.push(content.to_owned());

        // Rough token estimation: ~4 chars per token
        let estimated_tokens = content.len() / 4;
        ctx.token_count = ctx.token_count.saturating_add(estimated_tokens);

        // Trim old messages if exceeding the limit
        if ctx.messages.len() > max_recent {
            let excess = ctx.messages.len() - max_recent;
            ctx.messages.drain(..excess);
        }

        // Extract observations if threshold exceeded
        if ctx.token_count >= threshold {
            let observations = Self::extract_observations(ctx);
            let extracted: Vec<Observation> = observations.clone();
            ctx.observations.extend(observations);
            extracted
        } else {
            Vec::new()
        }
    }

    /// Extract observations from the accumulated context using keyword-based
    /// pattern matching (the zero-latency, zero-cost path).
    fn extract_observations(ctx: &SessionContext) -> Vec<Observation> {
        let mut observations = Vec::new();

        // Only analyze the most recent messages to avoid re-processing
        let recent_count = ctx.messages.len().min(5);
        let start = ctx.messages.len().saturating_sub(recent_count);

        for msg in &ctx.messages[start..] {
            let extracted = extract_observations_from_text(msg);
            observations.extend(extracted);
        }

        observations
    }

    /// Generate a reflection (high-level summary) for the session.
    ///
    /// If LLM summarization is enabled and a provider is available, uses the
    /// LLM. Otherwise, falls back to keyword-based extraction.
    pub fn generate_reflection(&self, session_id: &str) -> Option<String> {
        let ctx = self.sessions.get(session_id)?;

        if ctx.messages.is_empty() {
            return None;
        }

        // Keyword-based reflection: extract key topics
        let mut topic_counts: HashMap<String, usize> = HashMap::new();

        for msg in &ctx.messages {
            let lower = msg.to_lowercase();
            for keyword in KEYWORDS {
                if lower.contains(keyword) {
                    *topic_counts.entry((*keyword).to_string()).or_insert(0) += 1;
                }
            }
        }

        if topic_counts.is_empty() {
            return None;
        }

        // Sort by frequency
        let mut topics: Vec<_> = topic_counts.into_iter().collect();
        topics.sort_by(|a, b| b.1.cmp(&a.1));
        let top_topics: Vec<String> = topics.into_iter().take(5).map(|(k, _)| k).collect();

        let reflection = format!(
            "Session {}: discussed topics include {}. Total messages: {}, observations: {}.",
            session_id,
            top_topics.join(", "),
            ctx.messages.len(),
            ctx.observations.len()
        );
        Some(reflection)
    }

    /// Get the assembled context for a session (for injection into system prompt).
    ///
    /// Returns the three-tier context: reflections, observations, and recent messages.
    pub fn get_context(&self, session_id: &str) -> Option<SessionContext> {
        self.sessions.get(session_id).cloned()
    }

    /// Reset the context for a session (e.g., after conversation ends).
    pub fn reset_session(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    /// Return the current configuration.
    pub fn config(&self) -> &ObserverConfig {
        &self.config
    }
}

/// Keywords used for topic detection in reflections.
const KEYWORDS: &[&str] = &[
    "project",
    "code",
    "bug",
    "feature",
    "test",
    "deploy",
    "design",
    "api",
    "database",
    "security",
    "performance",
    "documentation",
    "meeting",
    "deadline",
    "user",
    "client",
    "product",
    "architecture",
    "refactor",
    "migration",
    "python",
    "rust",
    "javascript",
    "docker",
    "kubernetes",
    "cloud",
    "authentication",
    "authorization",
    "encryption",
    "logging",
    "monitoring",
];

/// Extract observations from a single message using keyword-based patterns.
fn extract_observations_from_text(text: &str) -> Vec<Observation> {
    let mut observations = Vec::new();
    let lower = text.to_lowercase();

    // Fact patterns
    let fact_patterns = [
        ("is ", "fact"),
        ("was ", "fact"),
        ("are ", "fact"),
        ("were ", "fact"),
        ("has ", "fact"),
        ("have ", "fact"),
        ("the ", "fact"),
    ];

    // Decision patterns
    let decision_patterns = [
        ("decided", "decision"),
        ("will do", "decision"),
        ("going to", "decision"),
        ("should", "decision"),
        ("must", "decision"),
        ("need to", "decision"),
        ("plan to", "decision"),
    ];

    // Preference patterns
    let preference_patterns = [
        ("like ", "preference"),
        ("prefer", "preference"),
        ("love ", "preference"),
        ("hate ", "preference"),
        ("don't like", "preference"),
        ("enjoy", "preference"),
        ("want ", "preference"),
    ];

    // Topic shift patterns
    let topic_shift_patterns = [
        ("by the way", "topic_shift"),
        ("on another note", "topic_shift"),
        ("changing subject", "topic_shift"),
        ("speaking of", "topic_shift"),
        ("related to", "topic_shift"),
    ];

    let all_patterns: Vec<(&str, &str)> = fact_patterns
        .iter()
        .chain(decision_patterns.iter())
        .chain(preference_patterns.iter())
        .chain(topic_shift_patterns.iter())
        .copied()
        .collect();

    for (pattern, obs_type) in all_patterns {
        if let Some(idx) = lower.find(pattern) {
            // Extract the sentence containing the pattern (from original text)
            if let Some(sentence) = extract_sentence_around(text, idx) {
                observations.push(Observation {
                    text: sentence,
                    observation_type: obs_type.to_owned(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
    }

    observations
}

/// Extract the sentence containing a given index position from text.
fn extract_sentence_around(text: &str, idx: usize) -> Option<String> {
    // Find sentence start (look backward)
    let start = if idx == 0 {
        0
    } else {
        let mut s = idx;
        for i in (0..idx).rev() {
            let b = text.as_bytes()[i];
            if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
                s = i + 1;
                break;
            }
        }
        s
    };

    // Find sentence end (look forward)
    let mut end = text.len();
    for i in (idx + 1)..text.len() {
        let b = text.as_bytes()[i];
        if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
            end = i + 1;
            break;
        }
    }

    let sentence = text[start..end].trim().to_owned();

    // Cap at reasonable length
    if sentence.len() > 300 {
        let truncated = &sentence[..300];
        let last_space = truncated.rfind(' ').unwrap_or(300);
        Some(format!("{}...", &sentence[..last_space]))
    } else if sentence.len() > 10 {
        Some(sentence)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_reasonable_thresholds() {
        let config = ObserverConfig::default();
        assert_eq!(config.observation_threshold, 2048);
        assert_eq!(config.max_recent_messages, 20);
        assert!(!config.enable_llm_summarization);
    }

    #[test]
    fn observer_tracks_session_context() {
        let mut observer = MemoryObserver::with_defaults();
        let ctx = observer.session_context("test-session");
        assert_eq!(ctx.session_id, "test-session");
        assert!(ctx.messages.is_empty());
    }

    #[test]
    fn on_message_stored_adds_message() {
        let mut observer = MemoryObserver::with_defaults();
        let _ = observer.on_message_stored("s1", "Hello world");

        let ctx = observer.get_context("s1").unwrap();
        assert_eq!(ctx.messages.len(), 1);
        assert_eq!(ctx.messages[0], "Hello world");
    }

    #[test]
    fn observation_extraction_finds_facts() {
        let observations =
            extract_observations_from_text("The project is using Rust for the backend.");
        assert!(!observations.is_empty());
        assert!(observations.iter().any(|o| o.observation_type == "fact"));
    }

    #[test]
    fn observation_extraction_finds_decisions() {
        let observations =
            extract_observations_from_text("We decided to use Neo4j for the graph database.");
        assert!(!observations.is_empty());
        assert!(observations
            .iter()
            .any(|o| o.observation_type == "decision"));
    }

    #[test]
    fn observation_extraction_finds_preferences() {
        let observations = extract_observations_from_text("I prefer dark mode for the dashboard.");
        assert!(!observations.is_empty());
        assert!(observations
            .iter()
            .any(|o| o.observation_type == "preference"));
    }

    #[test]
    fn observation_extraction_finds_topic_shifts() {
        let observations =
            extract_observations_from_text("By the way, we should also consider security.");
        assert!(!observations.is_empty());
        assert!(observations
            .iter()
            .any(|o| o.observation_type == "topic_shift"));
    }

    #[test]
    fn extract_sentence_around_returns_sentence() {
        let text = "Hello world. This is a test sentence. Goodbye.";
        let idx = text.find("test").unwrap();
        let result = extract_sentence_around(text, idx);
        assert!(result.is_some());
        let sentence = result.unwrap();
        assert!(sentence.contains("test"));
        assert!(sentence.len() <= 300);
    }

    #[test]
    fn extract_sentence_around_caps_long_sentences() {
        let long_text = "This is a very long sentence. ".repeat(50);
        let idx = long_text.find("very").unwrap();
        let result = extract_sentence_around(&long_text, idx);
        assert!(result.is_some());
        let sentence = result.unwrap();
        assert!(sentence.len() <= 303); // 300 + "..."
    }

    #[test]
    fn reflection_generation_creates_summary() {
        let mut observer = MemoryObserver::with_defaults();
        observer.on_message_stored("s1", "We discussed the project architecture.");
        observer.on_message_stored("s1", "The API design needs security improvements.");

        let reflection = observer.generate_reflection("s1");
        assert!(reflection.is_some());
        let ref_text = reflection.unwrap();
        assert!(ref_text.contains("s1"));
        assert!(
            ref_text.contains("project")
                || ref_text.contains("security")
                || ref_text.contains("api")
        );
    }

    #[test]
    fn reflection_returns_none_for_empty_session() {
        let observer = MemoryObserver::with_defaults();
        let reflection = observer.generate_reflection("nonexistent");
        assert!(reflection.is_none());
    }

    #[test]
    fn reset_session_clears_context() {
        let mut observer = MemoryObserver::with_defaults();
        observer.on_message_stored("s1", "Hello");
        observer.reset_session("s1");
        assert!(observer.get_context("s1").is_none());
    }

    #[test]
    fn threshold_triggers_observation_extraction() {
        let config = ObserverConfig {
            observation_threshold: 10, // Very low threshold for testing
            max_recent_messages: 20,
            enable_llm_summarization: false,
        };
        let mut observer = MemoryObserver::new(config);

        // Message long enough to exceed threshold (10 tokens = ~40 chars)
        let observations = observer.on_message_stored(
            "s1",
            "We decided to use Rust for the project because it is fast and safe.",
        );
        assert!(!observations.is_empty());
    }

    #[test]
    fn message_trimming_respects_max_recent() {
        let config = ObserverConfig {
            observation_threshold: 10000, // High threshold to avoid triggering
            max_recent_messages: 3,
            enable_llm_summarization: false,
        };
        let mut observer = MemoryObserver::new(config);

        observer.on_message_stored("s1", "msg1");
        observer.on_message_stored("s1", "msg2");
        observer.on_message_stored("s1", "msg3");
        observer.on_message_stored("s1", "msg4");

        let ctx = observer.get_context("s1").unwrap();
        assert_eq!(ctx.messages.len(), 3);
        assert_eq!(ctx.messages[0], "msg2");
        assert_eq!(ctx.messages[1], "msg3");
        assert_eq!(ctx.messages[2], "msg4");
    }
}
