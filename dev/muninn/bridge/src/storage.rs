//! Persistencia del spike: consultas fijas, scope del host y transacciones reales.

use chrono::{DateTime, Utc};
use neo4rs::{Graph, Query, Row, query};
use uuid::Uuid;

use crate::domain::{Conversation, Message, NewMessage, SessionInfo};
use crate::memory_search::{
    EMBEDDING_DIMENSIONS, EXACT_CANDIDATE_PROBE_LIMIT, MAX_EXACT_CANDIDATES,
    MESSAGE_EMBEDDING_INDEX, SEARCH_MESSAGES_BY_EMBEDDING, SEARCH_MESSAGES_EXACT,
    create_vector_index_query,
};

pub const FIXTURE_SCOPE: &str = "muninn-rust-spike";

const ADD_MESSAGE: &str = concat!(
    "MERGE (c:Conversation {scope:$scope,session_id:$session}) ",
    "ON CREATE SET c.id=$conversation,c.created_at=datetime($time) ",
    "SET c.sequence=coalesce(c.sequence,0)+1,c.updated_at=datetime($time) ",
    "WITH c ",
    "CALL (c) { OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(p:Message {scope:$scope}) ",
    "RETURN p ORDER BY p.sequence DESC LIMIT 1 } ",
    "CREATE (m:Message {scope:$scope,id:$id,role:$role,content:$content,",
    "timestamp:datetime($time),metadata:$metadata,sequence:c.sequence,embedding:$embedding}) ",
    "CREATE (c)-[:HAS_MESSAGE]->(m) ",
    "MERGE (e:MessageEmbedding {scope:$scope,message_id:$id,space_id:$space_id}) ",
    "SET e.vector=$embedding,e.dimensions=$dimensions,e.state='ready',",
    "e.created_at=coalesce(e.created_at,datetime($time)) ",
    "CREATE (m)-[:HAS_EMBEDDING {scope:$scope,message_id:$id}]->(e) ",
    "FOREACH (_ IN CASE WHEN p IS NULL THEN [1] ELSE [] END | CREATE (c)-[:FIRST_MESSAGE]->(m)) ",
    "FOREACH (_ IN CASE WHEN p IS NOT NULL THEN [1] ELSE [] END | CREATE (p)-[:NEXT_MESSAGE]->(m)) ",
    "RETURN m.id AS id"
);

fn owned_statement(suffix: &str) -> String {
    format!("MATCH (guard:MuninnBridgeGuard {{scope:$scope,owner:$owner}}) {suffix}")
}

const fn validate_embedding(embedding: &[f32]) -> Result<(), StorageError> {
    if embedding.len() != EMBEDDING_DIMENSIONS {
        return Err(StorageError::InvalidData);
    }
    Ok(())
}

fn validate_space_id(space_id: &str) -> Result<(), StorageError> {
    if space_id.is_empty()
        || space_id.len() > 128
        || !space_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        return Err(StorageError::InvalidData);
    }
    Ok(())
}

#[derive(Debug)]
pub enum StorageError {
    Unavailable,
    NotOwned,
    InvalidData,
    /// La consulta EXACT encontró más candidatos propietario/sesión que `MAX_EXACT_CANDIDATES`.
    ///
    /// Se rechaza en vez de puntuar un subconjunto silencioso.
    ExactCandidatesExceeded,
}

pub struct Store {
    graph: Graph,
    owner: String,
    #[cfg(test)]
    read_pause: Option<std::sync::Arc<tokio::sync::Barrier>>,
}

impl Store {
    pub async fn connect(password: &str, owner: String) -> Result<Self, StorageError> {
        let graph = Graph::new("neo4j:7687", "neo4j", password)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let store = Self {
            graph,
            owner,
            #[cfg(test)]
            read_pause: None,
        };
        let row = store
            .read_one(query("MATCH (n) RETURN count(n) AS total"))
            .await?;
        if row
            .get::<i64>("total")
            .map_err(|_| StorageError::InvalidData)?
            > 0
        {
            store.check_owner().await?;
            let row = store.read_one(query("MATCH (n) WHERE n.scope IS NULL OR n.scope <> $scope RETURN count(n) AS total")
                .param("scope", FIXTURE_SCOPE)).await?;
            if row
                .get::<i64>("total")
                .map_err(|_| StorageError::InvalidData)?
                != 0
            {
                return Err(StorageError::NotOwned);
            }
        }
        for ddl in [
            "CREATE CONSTRAINT muninn_spike_owner IF NOT EXISTS FOR (g:MuninnBridgeGuard) REQUIRE g.scope IS UNIQUE",
            "CREATE CONSTRAINT muninn_spike_conversation IF NOT EXISTS FOR (c:Conversation) REQUIRE (c.scope,c.session_id) IS UNIQUE",
            "CREATE CONSTRAINT muninn_spike_message IF NOT EXISTS FOR (m:Message) REQUIRE (m.scope,m.id) IS UNIQUE",
            "CREATE CONSTRAINT muninn_spike_message_embedding IF NOT EXISTS FOR (e:MessageEmbedding) REQUIRE (e.scope,e.message_id,e.space_id) IS UNIQUE",
        ] {
            store
                .graph
                .run(query(ddl))
                .await
                .map_err(|_| StorageError::Unavailable)?;
        }
        let row = store.write_one(query("MERGE (g:MuninnBridgeGuard {scope:$scope}) ON CREATE SET g.owner=$owner RETURN g.owner AS owner")
            .param("scope", FIXTURE_SCOPE).param("owner", store.owner.clone())).await?;
        if row
            .get::<String>("owner")
            .map_err(|_| StorageError::InvalidData)?
            != store.owner
        {
            return Err(StorageError::NotOwned);
        }
        Ok(store)
    }

    fn owned(&self, suffix: &str) -> Query {
        query(&owned_statement(suffix))
            .param("scope", FIXTURE_SCOPE)
            .param("owner", self.owner.clone())
    }

    async fn read_one(&self, statement: Query) -> Result<Row, StorageError> {
        let mut rows = self
            .graph
            .execute(statement)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let row = rows
            .next()
            .await
            .map_err(|_| StorageError::Unavailable)?
            .ok_or(StorageError::NotOwned)?;
        if rows
            .next()
            .await
            .map_err(|_| StorageError::Unavailable)?
            .is_some()
        {
            return Err(StorageError::InvalidData);
        }
        Ok(row)
    }

    async fn write_one(&self, statement: Query) -> Result<Row, StorageError> {
        let mut transaction = self
            .graph
            .start_txn()
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let result = async {
            let mut rows = transaction
                .execute(statement)
                .await
                .map_err(|_| StorageError::Unavailable)?;
            let row = rows
                .next(&mut transaction)
                .await
                .map_err(|_| StorageError::Unavailable)?
                .ok_or(StorageError::NotOwned)?;
            if rows
                .next(&mut transaction)
                .await
                .map_err(|_| StorageError::Unavailable)?
                .is_some()
            {
                return Err(StorageError::InvalidData);
            }
            Ok(row)
        }
        .await;
        match result {
            Ok(row) => {
                transaction
                    .commit()
                    .await
                    .map_err(|_| StorageError::Unavailable)?;
                Ok(row)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    pub async fn check_owner(&self) -> Result<(), StorageError> {
        self.read_one(self.owned("RETURN guard.owner AS owner"))
            .await?;
        Ok(())
    }

    pub async fn clear(&self) -> Result<(), StorageError> {
        self.write_one(self.owned("CALL (guard) { MATCH (n {scope:$scope}) WHERE NOT n:MuninnBridgeGuard DETACH DELETE n RETURN count(*) AS deleted } RETURN deleted")).await?;
        Ok(())
    }

    pub async fn sessions(&self, limit: u16) -> Result<Vec<SessionInfo>, StorageError> {
        let row = self.read_one(self.owned("\
            CALL (guard) { MATCH (c:Conversation {scope:$scope}) \
              OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message {scope:$scope}) \
              WITH c,count(m) AS total ORDER BY c.updated_at DESC,c.session_id LIMIT $limit \
              RETURN collect({session:c.session_id,total:total,created:toString(c.created_at),updated:toString(c.updated_at)}) AS sessions } \
            RETURN sessions").param("limit", i64::from(limit))).await?;
        #[derive(serde::Deserialize)]
        struct Record {
            session: String,
            total: u64,
            created: String,
            updated: String,
        }
        let records: Vec<Record> = row.get("sessions").map_err(|_| StorageError::InvalidData)?;
        records
            .into_iter()
            .map(|record| {
                Ok(SessionInfo {
                    session_id: record.session,
                    message_count: record.total,
                    created_at: parse_time(&record.created)?,
                    updated_at: parse_time(&record.updated)?,
                })
            })
            .collect()
    }

    pub async fn clear_session(&self, session: String) -> Result<(), StorageError> {
        self.write_one(
            self.owned(
                "\
            CALL (guard) { MATCH (c:Conversation {scope:$scope,session_id:$session}) \
              SET c.sequence=coalesce(c.sequence,0) \
              WITH c OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message {scope:$scope}) \
              WITH c,collect(m) AS messages \
              FOREACH (m IN messages | DETACH DELETE m) \
              DETACH DELETE c RETURN count(*) AS cleared } RETURN cleared",
            )
            .param("session", session),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_message(&self, id: Uuid) -> Result<bool, StorageError> {
        let row = self.write_one(self.owned("\
            OPTIONAL MATCH (c:Conversation {scope:$scope})-[:HAS_MESSAGE]->(m:Message {scope:$scope,id:$id}) \
            FOREACH (_ IN CASE WHEN c IS NULL THEN [] ELSE [1] END | SET c.sequence=coalesce(c.sequence,0)) \
            WITH c,m \
            OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(p:Message {scope:$scope})-[:NEXT_MESSAGE]->(m) \
            OPTIONAL MATCH (m)-[:NEXT_MESSAGE]->(n:Message {scope:$scope})<-[:HAS_MESSAGE]-(c) \
            OPTIONAL MATCH (c)-[first:FIRST_MESSAGE]->(m) \
            WITH c,m,p,n,first,m IS NOT NULL AS deleted \
            FOREACH (_ IN CASE WHEN p IS NOT NULL AND n IS NOT NULL THEN [1] ELSE [] END | MERGE (p)-[:NEXT_MESSAGE]->(n)) \
            FOREACH (_ IN CASE WHEN first IS NOT NULL AND n IS NOT NULL THEN [1] ELSE [] END | MERGE (c)-[:FIRST_MESSAGE]->(n)) \
            FOREACH (_ IN CASE WHEN m IS NULL THEN [] ELSE [1] END | DETACH DELETE m) \
            RETURN deleted").param("id", id.to_string())).await?;
        row.get("deleted").map_err(|_| StorageError::InvalidData)
    }

    pub async fn add(
        &self,
        input: NewMessage,
        embedding: &[f32],
        space_id: &str,
    ) -> Result<Message, StorageError> {
        validate_embedding(embedding)?;
        validate_space_id(space_id)?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        let metadata =
            serde_json::to_string(&input.metadata).map_err(|_| StorageError::InvalidData)?;
        let statement = self
            .owned(ADD_MESSAGE)
            .param("session", input.session_id)
            .param("conversation", Uuid::new_v4().to_string())
            .param("time", now.to_rfc3339())
            .param("id", id.to_string())
            .param("role", input.role.clone())
            .param("content", input.content.clone())
            .param("metadata", metadata)
            .param("embedding", embedding.to_vec())
            .param("space_id", space_id.to_owned())
            .param("dimensions", EMBEDDING_DIMENSIONS as i64);
        self.write_one(statement).await?;
        self.await_vector_index().await;
        Ok(Message {
            id,
            role: input.role,
            content: input.content,
            timestamp: now,
            metadata: input.metadata,
            embedding: Some(embedding.to_vec()),
        })
    }

    /// Crea el índice vectorial gestionado si no existe y espera a que quede
    /// en línea.
    ///
    /// El nombre, la etiqueta, la propiedad, las dimensiones y la función de
    /// similitud son los de la referencia fijada. Neo4j puebla el índice de
    /// forma asíncrona, así que un_search justo después del arranque no
    /// encontraría los mensajes ya escritos.
    pub async fn add_entity(
        &self,
        name: String,
        entity_type: String,
        description: Option<String>,
    ) -> Result<(Uuid, DateTime<Utc>), StorageError> {
        let id = Uuid::new_v4();
        let created_at = Utc::now();
        self.write_one(self.owned("CREATE (e:Entity {scope:$scope,id:$id,name:$name,type:$type,description:$description,canonical_name:$canonical,created_at:datetime($created)}) RETURN e.id AS id")
            .param("id", id.to_string()).param("name", name.clone()).param("type", entity_type)
            .param("description", description).param("canonical", name.to_lowercase())
            .param("created", created_at.to_rfc3339())).await?;
        Ok((id, created_at))
    }

    pub async fn add_preference(
        &self,
        category: String,
        preference: String,
        context: Option<String>,
    ) -> Result<Uuid, StorageError> {
        let id = Uuid::new_v4();
        self.write_one(self.owned("CREATE (p:Preference {scope:$scope,id:$id,category:$category,preference:$preference,context:$context}) RETURN p.id AS id")
            .param("id", id.to_string()).param("category", category).param("preference", preference)
            .param("context", context)).await?;
        Ok(id)
    }

    pub async fn add_fact(
        &self,
        subject: String,
        predicate: String,
        object: String,
    ) -> Result<Uuid, StorageError> {
        let id = Uuid::new_v4();
        self.write_one(self.owned("CREATE (f:Fact {scope:$scope,id:$id,subject:$subject,predicate:$predicate,object:$object}) RETURN f.id AS id")
            .param("id", id.to_string()).param("subject", subject).param("predicate", predicate)
            .param("object", object)).await?;
        Ok(id)
    }

    pub async fn ensure_vector_index(&self) -> Result<(), StorageError> {
        self.graph
            .run(query(&create_vector_index_query()))
            .await
            .map_err(|_| StorageError::Unavailable)?;
        for _ in 0..60 {
            let statement = query(
                "SHOW VECTOR INDEXES YIELD name, state \
                 WHERE name = $name RETURN state AS state",
            )
            .param("name", MESSAGE_EMBEDDING_INDEX);
            let mut rows = self
                .graph
                .execute(statement)
                .await
                .map_err(|_| StorageError::Unavailable)?;
            if let Some(row) = rows.next().await.map_err(|_| StorageError::Unavailable)? {
                let state: String = row.get("state").map_err(|_| StorageError::InvalidData)?;
                if state == "ONLINE" {
                    return Ok(());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err(StorageError::Unavailable)
    }

    /// Espera a que el índice vectorial incorpore lo escrito en la última
    /// transacción, para que la búsqueda sea determinista.
    async fn await_vector_index(&self) {
        let statement =
            query("CALL db.awaitIndexes($name, 5)").param("name", MESSAGE_EMBEDDING_INDEX);
        let _ = self.graph.run(statement).await;
    }

    /// Búsqueda semántica de mensajes por similitud coseno.
    ///
    /// Aplica el índice vectorial gestionado y el umbral, y filtra por sesión
    /// cuando el llamador la indica.
    pub async fn search_messages(
        &self,
        embedding: &[f32],
        session: Option<&str>,
        limit: u16,
        threshold: f64,
    ) -> Result<Vec<(Message, f64)>, StorageError> {
        validate_embedding(embedding)?;
        self.check_owner().await?;
        let statement = self
            .owned(SEARCH_MESSAGES_BY_EMBEDDING)
            .param("embedding", embedding.to_vec())
            .param("limit", i64::from(limit))
            .param("threshold", threshold)
            .param("session", session.map(str::to_owned));
        let mut rows = self
            .graph
            .execute(statement)
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let mut hits = Vec::new();
        while let Some(row) = rows.next().await.map_err(|_| StorageError::Unavailable)? {
            let record: std::collections::HashMap<String, String> =
                row.get("m").map_err(|_| StorageError::InvalidData)?;
            let embedding: Vec<f64> = row
                .get("embedding")
                .map_err(|_| StorageError::InvalidData)?;
            let score: f64 = row.get("score").map_err(|_| StorageError::InvalidData)?;
            let message = message_from_record(&record)?;
            hits.push((
                Message {
                    embedding: Some(embedding.into_iter().map(|value| value as f32).collect()),
                    ..message
                },
                score,
            ));
        }
        Ok(hits)
    }

    /// Búsqueda EXACT: coseno exhaustivo acotado por propietario/sesión, sin índice ANN.
    ///
    /// Baseline de recall para comparar contra `search_messages` (ANN).
    /// Filtra por sesión antes de puntuar cualquier candidato y rechaza
    /// (`ExactCandidatesExceeded`) en vez de truncar en silencio cuando el
    /// conjunto propietario/sesión supera `MAX_EXACT_CANDIDATES`.
    pub async fn search_messages_exact(
        &self,
        embedding: &[f32],
        session: Option<&str>,
        space_id: &str,
        limit: u16,
        threshold: f64,
    ) -> Result<Vec<(Message, f64)>, StorageError> {
        validate_embedding(embedding)?;
        validate_space_id(space_id)?;
        self.check_owner().await?;
        let statement = self
            .owned(SEARCH_MESSAGES_EXACT)
            .param("session_id", session.map(str::to_owned))
            .param("space_id", space_id.to_owned())
            .param("query_embedding", embedding.to_vec())
            .param("limit", i64::from(limit))
            .param("threshold", threshold)
            .param("max_candidates", i64::from(MAX_EXACT_CANDIDATES))
            .param("candidate_probe_limit", i64::from(EXACT_CANDIDATE_PROBE_LIMIT));
        let row = self.read_one(statement).await?;
        let over_limit: bool = row
            .get("over_limit")
            .map_err(|_| StorageError::InvalidData)?;
        if over_limit {
            return Err(StorageError::ExactCandidatesExceeded);
        }
        #[derive(serde::Deserialize)]
        struct Record {
            id: String,
            role: String,
            content: String,
            time: String,
            metadata: String,
            embedding: Vec<f64>,
            score: f64,
        }
        let records: Vec<Record> = row
            .get("results")
            .map_err(|_| StorageError::InvalidData)?;
        records
            .into_iter()
            .map(|record| {
                Ok((
                    Message {
                        id: Uuid::parse_str(&record.id).map_err(|_| StorageError::InvalidData)?,
                        role: record.role,
                        content: record.content,
                        timestamp: parse_time(&record.time)?,
                        metadata: serde_json::from_str(&record.metadata)
                            .map_err(|_| StorageError::InvalidData)?,
                        embedding: Some(
                            record
                                .embedding
                                .into_iter()
                                .map(|value| value as f32)
                                .collect(),
                        ),
                    },
                    record.score,
                ))
            })
            .collect()
    }

    pub async fn conversation(
        &self,
        session: String,
        limit: Option<u16>,
    ) -> Result<Conversation, StorageError> {
        #[cfg(test)]
        if let Some(pause) = &self.read_pause {
            pause.wait().await;
            pause.wait().await;
        }
        let requested = i64::from(limit.unwrap_or(1000));
        let row = self.read_one(self.owned("\
            OPTIONAL MATCH (c:Conversation {scope:$scope,session_id:$session}) \
            CALL (c) { OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message {scope:$scope}) \
              WITH m ORDER BY m.sequence LIMIT $limit \
              RETURN collect(CASE WHEN m IS NULL THEN null ELSE {id:m.id,role:m.role,content:m.content,time:toString(m.timestamp),metadata:m.metadata} END) AS messages } \
            RETURN c.id AS id,toString(c.created_at) AS created,messages")
            .param("session", session.clone()).param("limit", requested)).await?;
        let id: Option<String> = row.get("id").map_err(|_| StorageError::InvalidData)?;
        let Some(id) = id else {
            return Ok(Conversation {
                id: Uuid::new_v4(),
                session_id: session,
                created_at: Utc::now(),
                messages: Vec::new(),
            });
        };
        let created: String = row.get("created").map_err(|_| StorageError::InvalidData)?;
        let records: Vec<std::collections::HashMap<String, String>> =
            row.get("messages").map_err(|_| StorageError::InvalidData)?;
        let mut messages = Vec::new();
        for record in records {
            messages.push(message_from_record(&record)?);
        }
        Ok(Conversation {
            id: Uuid::parse_str(&id).map_err(|_| StorageError::InvalidData)?,
            session_id: session,
            created_at: parse_time(&created)?,
            messages,
        })
    }
}

/// Construye un mensaje desde la proyección `{id,role,content,time,metadata}`.
fn message_from_record(
    record: &std::collections::HashMap<String, String>,
) -> Result<Message, StorageError> {
    let value = |key: &str| {
        record
            .get(key)
            .map(String::as_str)
            .ok_or(StorageError::InvalidData)
    };
    Ok(Message {
        id: Uuid::parse_str(value("id")?).map_err(|_| StorageError::InvalidData)?,
        role: value("role")?.to_owned(),
        content: value("content")?.to_owned(),
        timestamp: parse_time(value("time")?)?,
        metadata: serde_json::from_str(value("metadata")?)
            .map_err(|_| StorageError::InvalidData)?,
        embedding: None,
    })
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| StorageError::InvalidData)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn stored_embeddings_require_the_canonical_dimensions() {
        assert!(matches!(
            validate_embedding(&[]),
            Err(StorageError::InvalidData)
        ));
        assert!(matches!(
            validate_embedding(&vec![0.0; EMBEDDING_DIMENSIONS - 1]),
            Err(StorageError::InvalidData)
        ));
        assert!(matches!(
            validate_embedding(&vec![0.0; EMBEDDING_DIMENSIONS + 1]),
            Err(StorageError::InvalidData)
        ));
        assert!(validate_embedding(&vec![0.0; EMBEDDING_DIMENSIONS]).is_ok());
    }

    #[test]
    fn a_space_id_must_be_a_plain_identifier() {
        assert!(matches!(
            validate_space_id(""),
            Err(StorageError::InvalidData)
        ));
        assert!(matches!(
            validate_space_id("espacio con espacios"),
            Err(StorageError::InvalidData)
        ));
        assert!(matches!(
            validate_space_id(&"a".repeat(129)),
            Err(StorageError::InvalidData)
        ));
        assert!(
            validate_space_id("48ae48b9951e502c6abdd5c29b63141f0d4fe49e701ef9a7393e978ab68caa6c")
                .is_ok()
        );
    }

    #[test]
    fn the_ann_search_statement_carries_the_owner_guard() {
        let statement = owned_statement(SEARCH_MESSAGES_BY_EMBEDDING);
        assert!(
            statement.starts_with("MATCH (guard:MuninnBridgeGuard {scope:$scope,owner:$owner})")
        );
        assert!(statement.contains(SEARCH_MESSAGES_BY_EMBEDDING));
        assert!(
            statement.contains(
                "db.index.vector.queryNodes('message_embedding_idx', $limit, $embedding)"
            )
        );
    }

    #[test]
    fn the_exact_search_statement_carries_the_owner_guard_before_any_candidate_match() {
        let statement = owned_statement(SEARCH_MESSAGES_EXACT);
        assert!(
            statement.starts_with("MATCH (guard:MuninnBridgeGuard {scope:$scope,owner:$owner})")
        );
        let guard_index = statement
            .find("MuninnBridgeGuard")
            .expect("guard match present");
        let candidate_match_index = statement
            .find("MATCH (c:Conversation")
            .expect("candidate match present");
        assert!(
            guard_index < candidate_match_index,
            "owner guard must be matched before any conversation/message access"
        );
    }

    #[test]
    fn the_add_statement_writes_both_embedding_representations() {
        let statement = owned_statement(ADD_MESSAGE);
        assert!(statement.contains("embedding:$embedding"));
        assert!(statement.contains("MessageEmbedding"));
        assert!(statement.contains("HAS_EMBEDDING"));
        assert!(statement.contains("space_id:$space_id"));
    }

    #[tokio::test]
    #[ignore = "Requiere Neo4j sintético aislado del harness Docker"]
    async fn guard_revocation_cannot_be_returned_as_an_empty_success() {
        let auth = std::fs::read_to_string("/run/secrets/neo4j_auth").expect("fixture auth file");
        let (_, password) = auth.trim().split_once('/').expect("fixture format");
        let token =
            std::fs::read_to_string("/run/secrets/bridge_token").expect("fixture token file");
        let owner = format!("{:x}", Sha256::digest(token.trim().as_bytes()));
        let mut store = Store::connect(password, owner)
            .await
            .expect("owned fixture");
        store.clear().await.expect("fixture cleanup");
        store
            .add(
                NewMessage {
                    session_id: "guard-gap".into(),
                    role: "user".into(),
                    content: "fixture".into(),
                    metadata: Default::default(),
                },
                &vec![0.0; EMBEDDING_DIMENSIONS],
                "48ae48b9951e502c6abdd5c29b63141f0d4fe49e701ef9a7393e978ab68caa6c",
            )
            .await
            .expect("fixture message");
        let pause = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        store.read_pause = Some(pause.clone());
        let graph = store.graph.clone();
        let revoker = tokio::spawn(async move {
            pause.wait().await;
            graph
                .run(
                    query("MATCH (g:MuninnBridgeGuard {scope:$scope}) SET g.owner='revoked'")
                        .param("scope", FIXTURE_SCOPE),
                )
                .await
                .expect("synthetic revocation");
            pause.wait().await;
        });
        let result = store.conversation("guard-gap".into(), None).await;
        revoker.await.expect("revoker completed");
        assert!(
            matches!(result, Err(StorageError::NotOwned)),
            "revoked fixture must not produce an empty successful conversation"
        );
    }

    /// Dataset determinista: vectores unitarios ortogonales en los primeros
    /// 32 ejes, para que el ranking por coseno exacto no tenga empates
    /// numéricos de punto flotante.
    #[cfg(test)]
    fn deterministic_unit_vector(axis: usize) -> Vec<f32> {
        let mut vector = vec![0.0f32; EMBEDDING_DIMENSIONS];
        vector[axis] = 1.0;
        vector
    }

    #[tokio::test]
    #[ignore = "Requiere Neo4j sintético aislado del harness Docker; compara EXACT con ANN, nunca la instancia activa"]
    async fn exact_search_matches_ann_recall_at_k_on_a_deterministic_synthetic_dataset() {
        let auth = std::fs::read_to_string("/run/secrets/neo4j_auth").expect("fixture auth file");
        let (_, password) = auth.trim().split_once('/').expect("fixture format");
        let token =
            std::fs::read_to_string("/run/secrets/bridge_token").expect("fixture token file");
        let owner = format!("{:x}", Sha256::digest(token.trim().as_bytes()));
        let store = Store::connect(password, owner)
            .await
            .expect("owned fixture");
        store.clear().await.expect("fixture cleanup");
        store.ensure_vector_index().await.expect("vector index");

        const SPACE_ID: &str = "48ae48b9951e502c6abdd5c29b63141f0d4fe49e701ef9a7393e978ab68caa6c";
        const AXES: usize = 32;
        const TOP_K: u16 = 10;

        // 32 mensajes con vectores unitarios ortogonales entre sí en la
        // misma sesión, de forma que el orden por coseno frente al eje 0 sea
        // determinista y sin empates: score 1.0 en el eje 0 y 0.0 en el resto.
        for axis in 0..AXES {
            store
                .add(
                    NewMessage {
                        session_id: "recall-session".into(),
                        role: "user".into(),
                        content: format!("axis-{axis}"),
                        metadata: Default::default(),
                    },
                    &deterministic_unit_vector(axis),
                    SPACE_ID,
                )
                .await
                .expect("fixture message");
        }
        // Distractores en otra sesión: no deben aparecer en ninguno de los
        // dos resultados porque el filtro de sesión se aplica antes de
        // puntuar.
        for axis in 0..AXES {
            store
                .add(
                    NewMessage {
                        session_id: "other-session".into(),
                        role: "user".into(),
                        content: format!("distractor-{axis}"),
                        metadata: Default::default(),
                    },
                    &deterministic_unit_vector(axis),
                    SPACE_ID,
                )
                .await
                .expect("fixture distractor");
        }

        let query_embedding = deterministic_unit_vector(0);

        let exact = store
            .search_messages_exact(
                &query_embedding,
                Some("recall-session"),
                SPACE_ID,
                TOP_K,
                0.0,
            )
            .await
            .expect("exact search");
        let ann = store
            .search_messages(&query_embedding, Some("recall-session"), TOP_K, 0.0)
            .await
            .expect("ann search");

        let exact_ids: std::collections::HashSet<_> =
            exact.iter().map(|(message, _)| message.id).collect();
        let ann_ids: std::collections::HashSet<_> =
            ann.iter().map(|(message, _)| message.id).collect();
        assert_eq!(
            exact.len(),
            usize::from(TOP_K),
            "exact must return exactly top-K on this bounded, duplicate-free dataset"
        );
        let overlap = exact_ids.intersection(&ann_ids).count();
        #[allow(clippy::cast_precision_loss)]
        let recall_at_k = overlap as f64 / exact_ids.len() as f64;
        // No se trata el recall como aprobado sólo por falta de entorno:
        // esta aserción exige igualdad exacta porque el dataset sintético no
        // tiene empates de score y ambas rutas comparten el mismo umbral e
        // índice vectorial coseno gestionado.
        assert!(
            (recall_at_k - 1.0).abs() < f64::EPSILON,
            "recall@{TOP_K} EXACT-vs-ANN = {recall_at_k} on a tie-free deterministic dataset, expected 1.0"
        );
        assert!(
            exact_ids
                .iter()
                .all(|id| store_message_content_contains_axis_prefix(&exact, *id)),
            "exact hits must come from the filtered session's own messages"
        );
    }

    #[cfg(test)]
    fn store_message_content_contains_axis_prefix(
        hits: &[(Message, f64)],
        id: Uuid,
    ) -> bool {
        hits.iter()
            .find(|(message, _)| message.id == id)
            .is_some_and(|(message, _)| message.content.starts_with("axis-"))
    }
}
