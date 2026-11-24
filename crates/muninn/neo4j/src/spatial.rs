//! Neo4j implementation of [`SpatialStore`] (`BOLT-08-03`, `BOLT-08-13`, `BOLT-08-14`).

use async_trait::async_trait;
use muninn_core::ports::{
    BoundingBox, GeoPoint, SessionLocation, SpatialLocationHit, SpatialStore,
};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::BoltType;
use uuid::Uuid;

use crate::bolt::{column, opt_string};
use crate::client::Neo4jClient;
use crate::long_term::entity_from_value;

#[derive(Clone)]
pub struct Neo4jSpatialStore {
    client: Neo4jClient,
}

impl Neo4jSpatialStore {
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }
}

fn validate_spatial_limit(limit: i64) -> MemoryResult<()> {
    if limit < 1 {
        return Err(MemoryError::invalid("spatial query limit must be >= 1"));
    }
    Ok(())
}

fn point_from_row(row: &neo4rs::Row) -> Option<GeoPoint> {
    let lat = row.get::<f64>("lat").ok()?;
    let lon = row.get::<f64>("lon").ok()?;
    GeoPoint::new(lat, lon).ok()
}

#[async_trait]
impl SpatialStore for Neo4jSpatialStore {
    async fn search_locations_near(
        &self,
        center: GeoPoint,
        radius_km: f64,
        limit: i64,
    ) -> MemoryResult<Vec<SpatialLocationHit>> {
        if !radius_km.is_finite() || radius_km < 0.0 {
            return Err(MemoryError::invalid("radius_km must be finite and >= 0.0"));
        }
        validate_spatial_limit(limit)?;
        let rows = self
            .client
            .execute(
                "MATCH (e:Entity) \
                 WHERE e.location IS NOT NULL \
                 WITH e, point.distance(e.location, point({latitude: $lat, longitude: $lon})) / 1000.0 AS distance_km \
                 WHERE distance_km <= $radius_km \
                 RETURN e, e.location.latitude AS lat, e.location.longitude AS lon, distance_km \
                 ORDER BY distance_km ASC, e.id ASC \
                 LIMIT $limit",
                vec![
                    ("lat", center.latitude.into()),
                    ("lon", center.longitude.into()),
                    ("radius_km", radius_km.into()),
                    ("limit", limit.into()),
                ],
            )
            .await?;

        let mut hits = Vec::with_capacity(rows.len());
        for row in &rows {
            let entity = entity_from_value(&column(row, "e")?)?;
            let coordinates = point_from_row(row)
                .ok_or_else(|| MemoryError::storage("spatial entity missing coordinates"))?;
            let distance_km = row.get::<f64>("distance_km").unwrap_or(0.0);
            hits.push(SpatialLocationHit {
                entity,
                coordinates,
                distance_km,
            });
        }
        Ok(hits)
    }

    async fn search_locations_in_bounding_box(
        &self,
        bbox: BoundingBox,
        limit: i64,
    ) -> MemoryResult<Vec<SpatialLocationHit>> {
        validate_spatial_limit(limit)?;
        let rows = self
            .client
            .execute(
                "MATCH (e:Entity) \
                 WHERE e.location IS NOT NULL \
                   AND point.withinBBox( \
                       e.location, \
                       point({latitude: $min_lat, longitude: $min_lon}), \
                       point({latitude: $max_lat, longitude: $max_lon}) \
                   ) \
                 RETURN e, e.location.latitude AS lat, e.location.longitude AS lon \
                 ORDER BY e.name ASC, e.id ASC \
                 LIMIT $limit",
                vec![
                    ("min_lat", bbox.min_lat.into()),
                    ("min_lon", bbox.min_lon.into()),
                    ("max_lat", bbox.max_lat.into()),
                    ("max_lon", bbox.max_lon.into()),
                    ("limit", limit.into()),
                ],
            )
            .await?;

        let mut hits = Vec::with_capacity(rows.len());
        for row in &rows {
            let entity = entity_from_value(&column(row, "e")?)?;
            let coordinates = point_from_row(row)
                .ok_or_else(|| MemoryError::storage("spatial entity missing coordinates"))?;
            hits.push(SpatialLocationHit {
                entity,
                coordinates,
                distance_km: 0.0,
            });
        }
        Ok(hits)
    }

    async fn get_location_coordinates(&self, entity_id: Uuid) -> MemoryResult<Option<GeoPoint>> {
        let rows = self
            .client
            .execute(
                "MATCH (e:Entity {id: $id}) \
                 WHERE e.location IS NOT NULL \
                 RETURN e.location.latitude AS lat, e.location.longitude AS lon \
                 LIMIT 1",
                vec![("id", entity_id.to_string().into())],
            )
            .await?;
        Ok(rows.first().and_then(point_from_row))
    }

    async fn get_locations(
        &self,
        session_id: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<SessionLocation>> {
        validate_spatial_limit(limit)?;
        let rows = self
            .client
            .execute(
                "MATCH (e:Entity) \
                 WHERE e.type = 'LOCATION' \
                 OPTIONAL MATCH (e)-[:EXTRACTED_FROM]->(m:Message)<-[:HAS_MESSAGE]-(c:Conversation) \
                 WHERE $session_id IS NULL OR c.session_id = $session_id \
                 WITH e, collect(DISTINCT c.id) AS conv_ids \
                 WHERE $session_id IS NULL OR size(conv_ids) > 0 \
                 RETURN e, \
                        CASE WHEN e.location IS NOT NULL THEN e.location.latitude ELSE null END AS lat, \
                        CASE WHEN e.location IS NOT NULL THEN e.location.longitude ELSE null END AS lon, \
                        conv_ids \
                 ORDER BY e.name ASC, e.id ASC \
                 LIMIT $limit",
                vec![
                    ("session_id", opt_string(session_id)),
                    ("limit", limit.into()),
                ],
            )
            .await?;

        let mut locations = Vec::with_capacity(rows.len());
        for row in &rows {
            let entity = entity_from_value(&column(row, "e")?)?;
            let coordinates = point_from_row(row);
            let conversation_ids = match column(row, "conv_ids")? {
                BoltType::List(items) => items
                    .value
                    .into_iter()
                    .filter_map(|v| match v {
                        BoltType::String(s) => Uuid::parse_str(&s.value).ok(),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            locations.push(SessionLocation {
                entity,
                coordinates,
                conversation_ids,
            });
        }
        Ok(locations)
    }
}
