//! Lightweight pattern-based preference detection from user messages.
//!
//! Equivalent to `neo4j_agent_memory.mcp._preference_detector`.
//!
//! Identifies preference statements like "I love Italian food" or "I don't
//! like spicy things" and extracts them with a category and sentiment.
//!
//! This is a rule-based classifier, not an LLM-based one. It trades recall
//! for zero latency and zero API cost, making it suitable for fire-and-forget
//! background processing after every user message.

/// A detected user preference.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedPreference {
    /// The raw text of the preference statement.
    pub text: String,
    /// Category: "food", "technology", "work", "entertainment", "general", etc.
    pub category: String,
    /// Sentiment: "positive" or "negative".
    pub sentiment: String,
    /// The raw preference (what the user likes or dislikes).
    pub preference: String,
}

/// Category keywords for classifying preferences.
const CATEGORY_KEYWORDS: &[(&str, &[&str])] = &[
    (
        "food",
        &[
            "food",
            "eat",
            "eating",
            "restaurant",
            "cuisine",
            "cook",
            "cooking",
            "meal",
            "dinner",
            "lunch",
            "breakfast",
            "snack",
            "drink",
            "coffee",
            "tea",
            "wine",
            "pizza",
            "sushi",
            "burger",
            "salad",
            "spicy",
            "sweet",
            "salty",
            "bitter",
            "Italian",
            "Mexican",
            "Chinese",
            "Japanese",
            "Indian",
            "Mediterranean",
        ],
    ),
    (
        "technology",
        &[
            "code",
            "programming",
            "software",
            "hardware",
            "computer",
            "laptop",
            "phone",
            "app",
            "application",
            "website",
            "browser",
            "IDE",
            "editor",
            "terminal",
            "Python",
            "Rust",
            "JavaScript",
            "TypeScript",
            "Go",
            "Java",
            "C++",
            "Linux",
            "macOS",
            "Windows",
            "Docker",
            "Kubernetes",
            "cloud",
            "API",
            "database",
            "Git",
            "GitHub",
            "VS Code",
            "Vim",
            "Neovim",
        ],
    ),
    (
        "work",
        &[
            "work",
            "job",
            "office",
            "meeting",
            "deadline",
            "project",
            "team",
            "colleague",
            "boss",
            "manager",
            "remote",
            "hybrid",
            "WFH",
            "schedule",
            "productivity",
            "task",
            "priority",
            "agile",
            "scrum",
            "standup",
            "presentation",
            "report",
            "email",
            "Slack",
            "Zoom",
        ],
    ),
    (
        "entertainment",
        &[
            "movie", "film", "show", "series", "book", "music", "game", "gaming", "sport",
            "exercise", "hobby", "travel", "vacation", "concert", "theater", "podcast", "YouTube",
            "Netflix", "Spotify", "anime", "comedy", "drama", "action", "horror", "sci-fi",
            "fantasy", "mystery",
        ],
    ),
    (
        "design",
        &[
            "design",
            "UI",
            "UX",
            "color",
            "font",
            "layout",
            "theme",
            "dark mode",
            "light mode",
            "minimal",
            "aesthetic",
            "visual",
            "graphic",
            "illustration",
            "photo",
            "photography",
            "art",
            "style",
            "typography",
        ],
    ),
    (
        "health",
        &[
            "health",
            "fitness",
            "gym",
            "workout",
            "yoga",
            "meditation",
            "sleep",
            "diet",
            "vegan",
            "vegetarian",
            "gluten",
            "allergy",
            "exercise",
            "run",
            "running",
            "swim",
            "bicycle",
            "cycling",
            "weight",
            "cardio",
        ],
    ),
];

/// Positive sentiment indicators.
const POSITIVE_PATTERNS: &[&str] = &[
    "love",
    "like",
    "enjoy",
    "prefer",
    "favorite",
    "fantastic",
    "great",
    "awesome",
    "amazing",
    "excellent",
    "good",
    "best",
    "really like",
    "really love",
    "adore",
    "appreciate",
    "fond of",
    "keen on",
    "into",
    "obsessed with",
    "can't get enough",
];

/// Negative sentiment indicators.
const NEGATIVE_PATTERNS: &[&str] = &[
    "hate",
    "dislike",
    "don't like",
    "don't love",
    "don't enjoy",
    "can't stand",
    "can't bear",
    "avoid",
    "annoying",
    "terrible",
    "worst",
    "bad",
    "awful",
    "boring",
    "ugly",
    "hate",
    "loathe",
    "dread",
    "sick of",
    "tired of",
    "not a fan",
    "not my thing",
    "not interested",
];

/// Preference extraction patterns (lowercase for matching against lowercased text).
const PREFERENCE_PATTERNS: &[&str] = &[
    "i love",
    "i like",
    "i enjoy",
    "i prefer",
    "my favorite",
    "i hate",
    "i dislike",
    "i don't like",
    "i don't enjoy",
    "i can't stand",
    "i avoid",
    "i'm into",
    "i'm obsessed with",
    "i appreciate",
    "i'm fond of",
    "i'm keen on",
    "i really like",
    "i really love",
    "i adore",
];

/// Detects preferences from user messages using pattern matching.
///
/// Equivalent to `PreferenceDetector` in the Python reference.
#[derive(Debug, Clone, Default)]
pub struct PreferenceDetector;

impl PreferenceDetector {
    /// Create a new preference detector.
    pub fn new() -> Self {
        Self
    }

    /// Detect preferences in a message.
    ///
    /// Returns a list of detected preferences, which may be empty if no
    /// preference patterns are found.
    pub fn detect(&self, text: &str) -> Vec<DetectedPreference> {
        let mut preferences = Vec::new();
        let lower = text.to_lowercase();

        // Check if this message contains any preference indicators
        // Check negative patterns first, because "don't like" contains "like"
        // which would also match positive patterns. Negative patterns are more
        // specific (longer), so we prioritize them.
        let has_negative = NEGATIVE_PATTERNS.iter().any(|p| lower.contains(p));
        let has_positive = POSITIVE_PATTERNS.iter().any(|p| lower.contains(p));

        if !has_positive && !has_negative {
            return preferences;
        }

        // Determine sentiment: negative takes precedence when both match
        // (e.g., "don't like" matches both "don't like" and "like")
        let sentiment = if has_negative { "negative" } else { "positive" };

        // Extract the preference text
        for pattern in PREFERENCE_PATTERNS {
            if let Some(idx) = lower.find(pattern) {
                // Extract the sentence containing the pattern
                if let Some(sentence) = extract_sentence(text, idx) {
                    let category = classify_category(&sentence);

                    // Clean up the preference text
                    let preference = clean_preference_text(&sentence, pattern);

                    preferences.push(DetectedPreference {
                        text: sentence,
                        category,
                        sentiment: sentiment.to_owned(),
                        preference,
                    });
                    break; // Only extract the first match per message
                }
            }
        }

        preferences
    }
}

/// Extract the sentence containing a given index.
fn extract_sentence(text: &str, idx: usize) -> Option<String> {
    // Find sentence start
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

    // Find sentence end
    let mut end = text.len();
    for i in (idx + 1)..text.len() {
        let b = text.as_bytes()[i];
        if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
            end = i + 1;
            break;
        }
    }

    let sentence = text[start..end].trim().to_owned();
    if sentence.len() > 10 {
        Some(sentence)
    } else {
        None
    }
}

/// Classify the category of a preference based on keywords.
fn classify_category(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut best_category = "general".to_owned();
    let mut best_score = 0;

    for (category, keywords) in CATEGORY_KEYWORDS {
        let score = keywords.iter().filter(|kw| lower.contains(**kw)).count();
        if score > best_score {
            best_score = score;
            best_category = (*category).to_string();
        }
    }

    best_category
}

/// Clean the preference text by removing the pattern prefix.
fn clean_preference_text(sentence: &str, pattern: &str) -> String {
    if let Some(idx) = sentence.to_lowercase().find(pattern) {
        let after = &sentence[idx + pattern.len()..];
        after.trim().to_owned()
    } else {
        sentence.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detector_finds_positive_preference() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I love Italian food for dinner.");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].sentiment, "positive");
        assert_eq!(prefs[0].category, "food");
    }

    #[test]
    fn detector_finds_negative_preference() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I don't like spicy food.");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].sentiment, "negative");
        assert_eq!(prefs[0].category, "food");
    }

    #[test]
    fn detector_classifies_technology_preference() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I prefer Rust for systems programming.");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].category, "technology");
    }

    #[test]
    fn detector_classifies_work_preference() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I enjoy remote work and flexible schedules.");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].category, "work");
    }

    #[test]
    fn detector_classifies_entertainment_preference() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I love sci-fi movies and anime.");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].category, "entertainment");
    }

    #[test]
    fn detector_returns_empty_for_no_preference() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("The weather is nice today.");
        assert!(prefs.is_empty());
    }

    #[test]
    fn detector_defaults_to_general_category() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I like the number seven.");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].category, "general");
    }

    #[test]
    fn clean_preference_text_removes_pattern() {
        let cleaned = clean_preference_text("i love Italian food.", "i love");
        assert_eq!(cleaned, "Italian food.");
    }

    #[test]
    fn classify_category_matches_food() {
        let category = classify_category("I really enjoy cooking Italian cuisine.");
        assert_eq!(category, "food");
    }

    #[test]
    fn classify_category_matches_health() {
        let category = classify_category("I love yoga and meditation every morning.");
        assert_eq!(category, "health");
    }

    #[test]
    fn classify_category_matches_design() {
        let category = classify_category("I prefer dark mode for the UI design.");
        assert_eq!(category, "design");
    }

    #[test]
    fn extract_sentence_returns_full_sentence() {
        let result = extract_sentence(
            "Hello world. I love programming in Rust. Goodbye.",
            17, // "I love" starts at 17
        );
        assert!(result.is_some());
        let sentence = result.unwrap();
        assert!(sentence.contains("love"));
        assert!(sentence.contains("Rust"));
    }

    #[test]
    fn detector_handles_multiple_patterns() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I really like Python and I really love Rust.");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].sentiment, "positive");
    }

    #[test]
    fn detector_case_insensitive() {
        let detector = PreferenceDetector::new();
        let prefs = detector.detect("I LOVE programming!");
        assert!(!prefs.is_empty());
        assert_eq!(prefs[0].sentiment, "positive");
    }
}
