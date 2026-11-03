//! Contrato de búsqueda semántica de memoria corto plazo.
//!
//! Réplica del contrato de la referencia fijada (SPEC-2.3) sobre el índice
//! vectorial `message_embedding_idx`. La referencia no filtra por sesión en la
//! ruta sin metadatos; aquí se filtra porque SPEC-2.3.2 lo exige. La
//! diferencia se documenta en `docs/muninn/rust-spike.md`.

use serde::{Deserialize, Serialize};

/// Índice vectorial gestionado, con el mismo nombre que la referencia fijada.
pub const MESSAGE_EMBEDDING_INDEX: &str = "message_embedding_idx";

/// Dimensiones del espacio de embeddings candidato (`minilm-cpu-f32-v1`).
pub const EMBEDDING_DIMENSIONS: usize = 384;

/// Límite superior de `limit`, alineado con el resto de la superficie del bridge.
pub const MAX_SEARCH_LIMIT: u16 = 1000;

/// Default de la referencia fijada.
pub const DEFAULT_LIMIT: u16 = 10;

/// Default de la referencia fijada.
pub const DEFAULT_THRESHOLD: f64 = 0.7;

/// Entrada de búsqueda, con los mismos nombres que el puente HTTP de la TCK.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInput {
    pub query: String,
    pub session_id: Option<String>,
    pub limit: Option<u16>,
    pub threshold: Option<f64>,
}

/// Entrada de búsqueda ya normalizada y validada.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRequest {
    pub query: String,
    pub session_id: Option<String>,
    pub limit: u16,
    pub threshold: f64,
}

impl SearchInput {
    /// Normaliza con los defaults de la referencia y rechaza entradas fuera de contrato.
    pub fn normalize(self) -> Result<SearchRequest, SearchError> {
        if self.query.trim().is_empty() {
            return Err(SearchError::InvalidQuery);
        }
        if self.query.len() > 4096 {
            return Err(SearchError::InvalidQuery);
        }
        if self.limit.is_some_and(|limit| limit > MAX_SEARCH_LIMIT) {
            return Err(SearchError::InvalidLimit);
        }
        if self.threshold.is_some_and(|threshold| !(0.0..=1.0).contains(&threshold)) {
            return Err(SearchError::InvalidThreshold);
        }
        if self.session_id.as_ref().is_some_and(|session| session.len() > 512) {
            return Err(SearchError::InvalidSession);
        }
        Ok(SearchRequest {
            query: self.query,
            session_id: self.session_id,
            limit: self.limit.unwrap_or(DEFAULT_LIMIT),
            threshold: self.threshold.unwrap_or(DEFAULT_THRESHOLD),
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SearchError {
    InvalidQuery,
    InvalidLimit,
    InvalidThreshold,
    InvalidSession,
}

/// Consulta de búsqueda por similitud, con filtro opcional por sesión.
///
/// El índice se consulta con `$limit` candidatos y el umbral se aplica
/// después, igual que en la referencia fijada: un resultado por debajo del
/// umbral no se rellena con el siguiente candidato.
pub const SEARCH_MESSAGES_BY_EMBEDDING: &str = "\
CALL db.index.vector.queryNodes('message_embedding_idx', $limit, $embedding) \
YIELD node, score \
WHERE node.scope = $scope AND score >= $threshold \
WITH node, score, [(node)<-[:HAS_MESSAGE]-(c:Conversation {scope:$scope}) | c.session_id] AS sessions \
WHERE $session IS NULL OR $session IN sessions \
RETURN {id:node.id,role:node.role,content:node.content,time:toString(node.timestamp),metadata:node.metadata} AS m, \
coalesce(node.embedding, []) AS embedding, score \
ORDER BY score DESC";

/// Límite superior fijo de candidatos exhaustivos aceptados por la consulta EXACT.
///
/// Deriva del diseño revisado `.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/minilm-exact-design-query-detailed.json`
/// (ver `.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/minilm-design-review-r2.json`, revisión PASS):
/// la consulta recoge como máximo `EXACT_CANDIDATE_PROBE_LIMIT` candidatos
/// propietario/sesión y rechaza explícitamente (`over_limit`) cuando el total
/// supera `MAX_EXACT_CANDIDATES`, sin truncado silencioso.
pub const MAX_EXACT_CANDIDATES: u32 = 10_000;

/// Límite de sondeo de candidatos.
///
/// Uno más que `MAX_EXACT_CANDIDATES`, igual que en el diseño revisado, para
/// poder distinguir "igual al límite" de "por encima del límite" sin
/// recolectar un conjunto no acotado.
pub const EXACT_CANDIDATE_PROBE_LIMIT: u32 = MAX_EXACT_CANDIDATES + 1;

/// Consulta EXACT: coseno exhaustivo acotado por propietario/sesión, sin índice ANN.
///
/// Replica el Cypher revisado y aprobado (PASS) en
/// `.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/minilm-exact-design-query-detailed.json`
/// (`cypher_sha256` `9a6b614ecb54e45505c1bad22d8345dd85a3ce60213fb0babba3745170f44d31`).
///
/// El filtro de sesión (`$session_id`) se aplica en la primera `CALL`, antes
/// de calcular cualquier score, igual que el filtro de propietario
/// (`guard:MuninnBridgeGuard`) aplicado por la consulta completa. La segunda
/// `CALL` sólo puntúa cuando el número de candidatos no excede
/// `$max_candidates`; en caso contrario `over_limit` es verdadero y
/// `results` queda vacío, sin aceptar un corte parcial silencioso.
pub const SEARCH_MESSAGES_EXACT: &str = "\
CALL (guard) { \
MATCH (c:Conversation {scope:$scope})-[:HAS_MESSAGE]->(m:Message {scope:$scope}) \
WHERE $session_id IS NULL OR c.session_id = $session_id \
MATCH (m)-[:HAS_EMBEDDING]->(e:MessageEmbedding {scope:$scope,space_id:$space_id}) \
WHERE e.message_id = m.id AND e.vector IS NOT NULL \
WITH DISTINCT m, e \
LIMIT $candidate_probe_limit \
RETURN collect({message: m, embedding: e}) AS candidates \
} \
CALL (candidates) { \
WITH candidates WHERE size(candidates) <= $max_candidates \
UNWIND candidates AS candidate \
WITH candidate.message AS m, candidate.embedding AS e \
WITH m, vector.similarity.cosine(e.vector, $query_embedding) AS score \
WHERE score >= $threshold \
WITH m, score ORDER BY score DESC, m.id ASC LIMIT $limit \
RETURN collect({id:m.id,role:m.role,content:m.content,time:toString(m.timestamp),metadata:m.metadata,embedding:coalesce(m.embedding, []),score:score}) AS results \
} \
RETURN size(candidates) AS candidate_count, \
size(candidates) > $max_candidates AS over_limit, \
results";

/// Creación del índice vectorial gestionado, con la misma función de similitud
/// y las mismas dimensiones que declara la referencia fijada.
pub fn create_vector_index_query() -> String {
    format!(
        "CREATE VECTOR INDEX {MESSAGE_EMBEDDING_INDEX} IF NOT EXISTS \
FOR (n:Message) ON (n.embedding) \
OPTIONS {{indexConfig: {{`vector.dimensions`: {EMBEDDING_DIMENSIONS}, \
`vector.similarity_function`: 'cosine'}}}}"
    )
}

/// Resultado de búsqueda: mensaje y similitud.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub id: uuid::Uuid,
    pub role: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
    pub embedding: Option<Vec<f32>>,
    pub score: f64,
}

impl SearchHit {
    pub fn from_message(message: crate::domain::Message, score: f64) -> Self {
        Self {
            id: message.id,
            role: message.role,
            content: message.content,
            timestamp: message.timestamp,
            metadata: message.metadata,
            embedding: message.embedding,
            score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(query: &str) -> SearchInput {
        SearchInput {
            query: query.to_owned(),
            session_id: None,
            limit: None,
            threshold: None,
        }
    }

    #[test]
    fn applies_the_reference_defaults() {
        let request = input("hola").normalize().unwrap();
        assert_eq!(request.limit, DEFAULT_LIMIT);
        assert_eq!(request.threshold, DEFAULT_THRESHOLD);
    }

    #[test]
    fn rejects_an_empty_query() {
        assert_eq!(input("   ").normalize(), Err(SearchError::InvalidQuery));
    }

    #[test]
    fn rejects_a_limit_above_the_bridge_bound() {
        let mut value = input("hola");
        value.limit = Some(MAX_SEARCH_LIMIT + 1);
        assert_eq!(value.normalize(), Err(SearchError::InvalidLimit));
    }

    #[test]
    fn rejects_a_threshold_outside_the_cosine_range() {
        let mut value = input("hola");
        value.threshold = Some(1.5);
        assert_eq!(value.normalize(), Err(SearchError::InvalidThreshold));
        let mut value = input("hola");
        value.threshold = Some(-0.1);
        assert_eq!(value.normalize(), Err(SearchError::InvalidThreshold));
    }

    #[test]
    fn keeps_the_pinned_index_name_and_similarity_contract() {
        let query = create_vector_index_query();
        assert!(query.contains("message_embedding_idx"));
        assert!(query.contains("FOR (n:Message) ON (n.embedding)"));
        assert!(query.contains("`vector.dimensions`: 384"));
        assert!(query.contains("`vector.similarity_function`: 'cosine'"));
    }

    #[test]
    fn search_query_applies_the_threshold_after_the_index_candidates() {
        assert!(SEARCH_MESSAGES_BY_EMBEDDING.contains("db.index.vector.queryNodes('message_embedding_idx', $limit, $embedding)"));
        assert!(SEARCH_MESSAGES_BY_EMBEDDING.contains("score >= $threshold"));
        assert!(SEARCH_MESSAGES_BY_EMBEDDING.contains("node.scope = $scope"));
        assert!(SEARCH_MESSAGES_BY_EMBEDDING.contains("ORDER BY score DESC"));
        assert!(SEARCH_MESSAGES_BY_EMBEDDING.contains("$session IS NULL OR $session IN sessions"));
    }

    #[test]
    fn exact_search_candidate_probe_limit_is_one_more_than_the_bound() {
        assert_eq!(EXACT_CANDIDATE_PROBE_LIMIT, MAX_EXACT_CANDIDATES + 1);
    }

    #[test]
    fn exact_search_query_filters_session_before_scoring_any_candidate() {
        let session_filter_index = SEARCH_MESSAGES_EXACT
            .find("$session_id IS NULL OR c.session_id = $session_id")
            .expect("session filter present");
        let scoring_index = SEARCH_MESSAGES_EXACT
            .find("vector.similarity.cosine")
            .expect("cosine scoring present");
        assert!(
            session_filter_index < scoring_index,
            "session must be filtered before any cosine score is computed"
        );
    }

    #[test]
    fn exact_search_query_rejects_over_limit_candidate_sets_instead_of_truncating() {
        assert!(SEARCH_MESSAGES_EXACT.contains("WHERE size(candidates) <= $max_candidates"));
        assert!(SEARCH_MESSAGES_EXACT.contains("size(candidates) > $max_candidates AS over_limit"));
    }

    #[test]
    fn exact_search_query_scopes_candidates_to_the_canonical_space_id() {
        assert!(SEARCH_MESSAGES_EXACT.contains("space_id:$space_id"));
        assert!(SEARCH_MESSAGES_EXACT.contains("e.message_id = m.id AND e.vector IS NOT NULL"));
    }

    #[test]
    fn exact_search_query_breaks_score_ties_by_message_id() {
        assert!(SEARCH_MESSAGES_EXACT.contains("ORDER BY score DESC, m.id ASC LIMIT $limit"));
    }
}
