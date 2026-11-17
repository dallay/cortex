//! Manifest-bound redirect authority, not a downloader or DNS enforcement layer.

use crate::model_manifest::PinnedMiniLmManifest;
use url::Url;
#[path = "artifact_http.rs"]
pub mod http;

#[derive(Debug, PartialEq, Eq)]
pub enum RouteError {
    UnknownArtifact,
    Rejected,
    RedirectLimit,
    Cycle,
}

/// No Debug/Display: signed query strings must not enter diagnostics.
pub struct ArtifactRoute {
    current: Url,
    source_path: String,
    cache_path: String,
    encoded_cache_path: Option<String>,
    weight: bool,
    redirects: u8,
    visited: Vec<Url>,
}

impl ArtifactRoute {
    pub fn new(manifest: &PinnedMiniLmManifest, path: &str) -> Result<Self, RouteError> {
        let artifact = manifest
            .files()
            .iter()
            .find(|artifact| artifact.path() == path)
            .ok_or(RouteError::UnknownArtifact)?;
        let current = Url::parse(artifact.source_url()).map_err(|_| RouteError::Rejected)?;
        let source_path = current.path().to_owned();
        let cache_path = format!(
            "/api/resolve-cache/models/{}/{}/{path}",
            crate::model_manifest::MODEL_ID,
            manifest.revision()
        );
        // Hub encodes the declared artifact-relative slash inside resolve-cache.
        // Match this exact derived variant; never decode arbitrary incoming paths.
        let encoded_cache_path = path.contains('/').then(|| {
            format!(
                "/api/resolve-cache/models/{}/{}/{}",
                crate::model_manifest::MODEL_ID,
                manifest.revision(),
                path.replace('/', "%2F")
            )
        });
        let visited = vec![current.clone()];
        Ok(Self {
            current,
            source_path,
            cache_path,
            encoded_cache_path,
            weight: path == "model.safetensors",
            redirects: 0,
            visited,
        })
    }

    pub fn follow(&mut self, location: &str) -> Result<(), RouteError> {
        if self.current.host_str() != Some("huggingface.co") {
            return Err(RouteError::Rejected);
        }
        if self.redirects >= 4 {
            return Err(RouteError::RedirectLimit);
        }
        if location.is_empty()
            || location.len() > 16 * 1024
            || !location
                .bytes()
                .all(|byte| matches!(byte, 0x21..=0x7e) && byte != b'\\')
            || location.starts_with("///")
            || !(location.starts_with('/')
                || location.starts_with("https://")
                || location.starts_with('?'))
        {
            return Err(RouteError::Rejected);
        }
        if location
            .strip_prefix("https://")
            .or_else(|| location.strip_prefix("//"))
            .is_some_and(|rest| {
                rest.split(['/', '?', '#'])
                    .next()
                    .is_some_and(|authority| authority.is_empty() || authority.contains('@'))
            })
        {
            return Err(RouteError::Rejected);
        }
        let target = self
            .current
            .join(location)
            .map_err(|_| RouteError::Rejected)?;
        // URL parsing can percent-encode otherwise printable query characters.
        // Signed query bytes must survive unchanged or the transition is rejected.
        if target.query() != location.split_once('?').map(|(_, query)| query) {
            return Err(RouteError::Rejected);
        }
        if target.as_str().len() > 16 * 1024
            || target.scheme() != "https"
            || target.port_or_known_default() != Some(443)
            || !target.username().is_empty()
            || target.password().is_some()
            || target.fragment().is_some()
        {
            return Err(RouteError::Rejected);
        }
        let allowed = match target.host_str() {
            Some("huggingface.co") => {
                target.path() == self.source_path
                    || target.path() == self.cache_path
                    || self.encoded_cache_path.as_deref() == Some(target.path())
            }
            Some("us.aws.cdn.hf.co") if self.weight => valid_cdn_path(target.path()),
            _ => false,
        };
        if !allowed {
            return Err(RouteError::Rejected);
        }
        if self.visited.contains(&target) {
            return Err(RouteError::Cycle);
        }
        self.visited.push(target.clone());
        self.current = target;
        self.redirects += 1;
        Ok(())
    }
}

fn valid_cdn_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/xet-bridge-us/") else {
        return false;
    };
    let Some((repository, object)) = rest.split_once('/') else {
        return false;
    };
    crate::artifacts::is_lower_hex(repository, 24) && crate::artifacts::is_lower_hex(object, 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_a_pinned_manifest_artifact() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let route = ArtifactRoute::new(&manifest, "config.json");
        assert!(route.is_ok());
        let route = route.expect("pinned artifact");
        assert_eq!(route.current.scheme(), "https");
        assert_eq!(route.current.host_str(), Some("huggingface.co"));
        assert!(route.current.path().contains(manifest.revision()));
        for path in ["../escape", "remote.py", "https://evil.invalid/"] {
            assert!(matches!(
                ArtifactRoute::new(&manifest, path),
                Err(RouteError::UnknownArtifact)
            ));
        }
    }

    fn cache_path(manifest: &PinnedMiniLmManifest, path: &str) -> String {
        format!(
            "/api/resolve-cache/models/{}/{}/{path}",
            crate::model_manifest::MODEL_ID,
            manifest.revision()
        )
    }

    #[test]
    fn allows_the_same_artifacts_immutable_hub_cache_path() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        let target = format!(
            "{}?download=true&etag=fixture",
            cache_path(&manifest, "config.json")
        );
        assert!(route.follow(&target).is_ok());
        assert_eq!(route.current.path(), cache_path(&manifest, "config.json"));
        assert_eq!(route.current.query(), Some("download=true&etag=fixture"));
    }

    fn cdn_location() -> String {
        format!("https://us.aws.cdn.hf.co/xet-bridge-us/{}/{}?Policy=synthetic%2Bopaque&Signature=fixture%2Fvalue&Key-Pair-Id=fixture", "a".repeat(24), "b".repeat(64))
    }

    #[test]
    fn permits_an_explicit_terminal_weight_cdn_and_preserves_opaque_query_bytes() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "model.safetensors").expect("pinned weight");
        let location = cdn_location();
        assert!(route.follow(&location).is_ok());
        assert_eq!(route.current.as_str(), location);
        assert_eq!(route.follow(&location), Err(RouteError::Rejected));
        assert_eq!(
            route.follow("https://huggingface.co/"),
            Err(RouteError::Rejected)
        );
    }

    #[test]
    fn permits_four_redirects_then_rejects_without_mutating_the_current_url() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        for index in 1..=4 {
            route.follow(&format!("?hop={index}")).expect("allowed hop");
        }
        let before = route.current.clone();
        assert_eq!(route.follow("?hop=5"), Err(RouteError::RedirectLimit));
        assert_eq!(route.current, before);
    }

    #[test]
    fn detects_cycles_including_a_return_to_the_initial_url() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        let initial = route.current.clone();
        route
            .follow(&cache_path(&manifest, "config.json"))
            .expect("cache hop");
        let before = route.current.clone();
        assert_eq!(route.follow(initial.as_str()), Err(RouteError::Cycle));
        assert_eq!(route.current, before);
        assert_eq!(route.follow(before.as_str()), Err(RouteError::Cycle));
        assert_eq!(route.redirects, 1);
    }

    #[test]
    fn rejects_even_empty_userinfo_in_an_absolute_authority() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        let target = format!("https://@huggingface.co{}?fixture=1", route.source_path);
        assert_eq!(route.follow(&target), Err(RouteError::Rejected));
        assert_eq!(route.redirects, 0);
    }

    #[test]
    fn rejects_missing_authority_instead_of_normalizing_it() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        let target = format!("https:///huggingface.co{}?fixture=1", route.source_path);
        assert_eq!(route.follow(&target), Err(RouteError::Rejected));
    }

    #[test]
    fn unsafe_authorities_schemes_and_raw_forms_leave_the_route_unchanged() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        let initial = route.current.clone();
        let path = route.source_path.clone();
        for base in [
            "http://huggingface.co",
            "https://evil.invalid",
            "https://huggingface.co.evil.invalid",
            "https://huggingface.co.",
            "https://127.0.0.1",
            "https://[::1]",
            "https://huggingface.co:444",
            "https://user:password@huggingface.co",
            "https://huggingface.co@evil.invalid",
            "//evil.invalid",
        ] {
            assert_eq!(
                route.follow(&format!("{base}{path}?test=1")),
                Err(RouteError::Rejected)
            );
            assert_eq!(route.current, initial);
            assert_eq!(route.redirects, 0);
        }
        for location in [
            "",
            " https://huggingface.co",
            "https:\\huggingface.co",
            "\n?query=x",
            "?raw space",
            "?é",
            "#fragment",
            "///huggingface.co",
            "javascript:alert(1)",
        ] {
            assert_eq!(route.follow(location), Err(RouteError::Rejected));
        }
        assert_eq!(
            route.follow(&format!(
                "{}#fragment",
                cache_path(&manifest, "config.json")
            )),
            Err(RouteError::Rejected)
        );
        assert_eq!(route.current, initial);
        assert_eq!(route.visited.len(), 1);
    }

    #[test]
    fn rejects_other_artifacts_revisions_models_and_encoded_path_separators() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        let expected = cache_path(&manifest, "config.json");
        for target in [
            cache_path(&manifest, "tokenizer.json"),
            expected.replace(manifest.revision(), "main"),
            expected.replace(crate::model_manifest::MODEL_ID, "different/model"),
            expected.replace("config.json", "nested%2Fconfig.json"),
            expected.replace("config.json", "config.json%00"),
            expected.replace("models/", "models%2F"),
            expected.replace("config.json", "%63onfig.json"),
        ] {
            assert_eq!(route.follow(&target), Err(RouteError::Rejected));
        }
        assert_eq!(route.redirects, 0);
    }

    #[test]
    fn cdn_is_weight_only_and_rejects_other_regions_path_shapes_and_ports() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let location = cdn_location();
        let mut config = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        assert_eq!(config.follow(&location), Err(RouteError::Rejected));
        let mut weight = ArtifactRoute::new(&manifest, "model.safetensors").expect("pinned weight");
        for target in [
            location.replace("us.aws", "eu.aws"),
            location.replace("hf.co/", "hf.co.evil.invalid/"),
            location.replace("hf.co/", "hf.co:444/"),
            location.replace("xet-bridge-us", "xet-bridge-eu"),
            location.replace(&"a".repeat(24), &"a".repeat(23)),
            location.replace(&"b".repeat(64), &"B".repeat(64)),
            location.replace("?Policy", "/extra?Policy"),
            format!("{location}#fragment"),
        ] {
            assert_eq!(weight.follow(&target), Err(RouteError::Rejected));
        }
        assert_eq!(weight.redirects, 0);
        assert!(weight.follow(&location).is_ok());
        let before = weight.current.clone();
        assert_eq!(
            weight.follow(&format!("{}?again=1", weight.source_path)),
            Err(RouteError::Rejected)
        );
        assert_eq!(weight.current, before);
    }

    #[test]
    fn bounds_both_raw_location_and_resolved_url_lengths() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "config.json").expect("pinned artifact");
        let prefix_length = route.current.as_str().len() + 1;
        let query = "a".repeat(16 * 1024 - prefix_length);
        route
            .follow(&format!("?{query}"))
            .expect("exact resolved URL bound");
        assert_eq!(route.current.as_str().len(), 16 * 1024);
        let before = route.current.clone();
        assert_eq!(
            route.follow(&format!("?{}", "b".repeat(query.len() + 1))),
            Err(RouteError::Rejected)
        );
        assert_eq!(
            route.follow(&"a".repeat(16 * 1024 + 1)),
            Err(RouteError::Rejected)
        );
        assert_eq!(route.current, before);
        assert_eq!(route.redirects, 1);
    }

    #[test]
    fn diagnostic_errors_do_not_include_synthetic_signed_query_values() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "model.safetensors").expect("pinned weight");
        let location = cdn_location().replace("us.aws.cdn.hf.co", "evil.invalid");
        let error = route.follow(&location).expect_err("unapproved destination");
        assert_eq!(format!("{error:?}"), "Rejected");
    }

    #[test]
    fn rejects_query_normalization_instead_of_rewriting_a_signature() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route = ArtifactRoute::new(&manifest, "model.safetensors").expect("pinned weight");
        let before = route.current.clone();
        let location = cdn_location().replace("Signature=fixture%2Fvalue", "Signature=a'b");
        assert_eq!(route.follow(&location), Err(RouteError::Rejected));
        assert_eq!(route.current, before);
        assert_eq!(route.redirects, 0);
    }

    #[tokio::test]
    async fn dependency_selection_does_not_make_existing_neo4j_tls_initialization_panic() {
        // No live Neo4j or TLS server is required: only initialize the existing
        // driver. Port zero cannot serve this fixture; network success is not tested.
        let result = tokio::spawn(async {
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                neo4rs::Graph::new(
                    "bolt+s://127.0.0.1:0",
                    "synthetic-fixture",
                    "synthetic-fixture",
                ),
            )
            .await
        })
        .await;
        assert!(
            result.is_ok(),
            "existing Neo4j TLS initialization must not panic"
        );
    }

    #[test]
    fn accepts_only_the_exact_declared_nested_artifacts_encoded_cache_path() {
        let manifest = PinnedMiniLmManifest::synthetic_for_tests();
        let mut route =
            ArtifactRoute::new(&manifest, "1_Pooling/config.json").expect("pinned nested artifact");
        let target = format!(
            "{}?etag=synthetic%22opaque%22",
            cache_path(&manifest, "1_Pooling%2Fconfig.json")
        );
        assert_eq!(route.follow(&target), Ok(()));
        assert_eq!(
            route.current.path(),
            cache_path(&manifest, "1_Pooling%2Fconfig.json")
        );
        assert_eq!(route.current.query(), Some("etag=synthetic%22opaque%22"));
        for path in [
            "1_Pooling%252Fconfig.json",
            "..%2Fconfig.json",
            "1_Pooling%2Ftokenizer.json",
            "1_Pooling%2fconfig.json",
        ] {
            let mut other =
                ArtifactRoute::new(&manifest, "1_Pooling/config.json").expect("nested artifact");
            assert_eq!(
                other.follow(&cache_path(&manifest, path)),
                Err(RouteError::Rejected)
            );
        }
        let mut unrelated = ArtifactRoute::new(&manifest, "config.json").expect("root artifact");
        assert_eq!(unrelated.follow(&target), Err(RouteError::Rejected));
    }
}
