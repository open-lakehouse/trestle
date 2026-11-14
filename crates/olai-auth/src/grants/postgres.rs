use async_trait::async_trait;
use sqlx::PgPool;

use super::{
    DEFAULT_SEPARATOR, Grant, GrantChange, GrantError, GrantStore, GroupMembership, ObjectRef,
    Subject, SubjectFacts, SubjectKind, check_changes, check_rename,
};
use crate::postgres::{DEFAULT_SCHEMA, quote_ident};

/// A [`GrantStore`] over the `grants`, `groups` and `memberships` tables of the `authz`
/// migrations ([`migrate`](crate::migrate)), in the `authz` schema unless configured
/// otherwise.
///
/// Reads need `SELECT` on the three tables; [`apply`](GrantStore::apply),
/// [`delete_object`](GrantStore::delete_object) and [`rename`](GrantStore::rename) need
/// `INSERT` and `DELETE` on `grants`. Groups and memberships are managed outside this
/// store.
///
/// sqlx is linked with only its `postgres` feature: the host enables a runtime and
/// TLS backend for its own pool.
#[derive(Debug, Clone)]
pub struct PgGrantStore {
    pool: PgPool,
    schema: String,
    separator: String,
}

impl PgGrantStore {
    /// A store over the `authz` schema through `pool`.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            schema: quote_ident(DEFAULT_SCHEMA),
            separator: DEFAULT_SEPARATOR.to_owned(),
        }
    }

    /// Reads and writes the tables in `schema` instead of `authz`.
    pub fn with_schema(mut self, schema: &str) -> Self {
        self.schema = quote_ident(schema);
        self
    }

    /// Separates the parts of hierarchical object ids with `separator` instead of `.`.
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }

    fn list_query(&self) -> String {
        format!(
            "SELECT subject_type, subject_id, privilege FROM {}.grants \
             WHERE object_type = $1 AND object_id = $2",
            self.schema
        )
    }

    /// The condition on `object_type`/`object_id` matching the object (`$1`, `$2`) and its
    /// descendants of the types `$3` (separator `$4`).
    const AFFECTED: &str = "((object_type = $1 AND object_id = $2) \
         OR (object_type = ANY($3) AND starts_with(object_id, $2 || $4) \
             AND length(object_id) > length($2 || $4)))";
}

fn store_error(e: sqlx::Error) -> GrantError {
    GrantError::Store(Box::new(e))
}

fn parse_subject(kind: &str, id: String) -> Result<Subject, GrantError> {
    let kind = SubjectKind::parse(kind)
        .ok_or_else(|| GrantError::Store(format!("unknown subject type {kind:?}").into()))?;
    Ok(Subject::new(kind, id))
}

fn grants_of(
    object: &ObjectRef,
    rows: Vec<(String, String, String)>,
) -> Result<Vec<Grant>, GrantError> {
    let mut grants = rows
        .into_iter()
        .map(|(kind, id, privilege)| {
            Ok(Grant {
                subject: parse_subject(&kind, id)?,
                privilege,
                object: object.clone(),
            })
        })
        .collect::<Result<Vec<_>, GrantError>>()?;
    // In `Subject`'s order, as every store sorts.
    grants.sort();
    Ok(grants)
}

#[async_trait]
impl GrantStore for PgGrantStore {
    async fn list(&self, object: &ObjectRef) -> Result<Vec<Grant>, GrantError> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(&self.list_query())
            .bind(&object.entity_type)
            .bind(&object.id)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
        grants_of(object, rows)
    }

    async fn apply(
        &self,
        object: &ObjectRef,
        changes: &[GrantChange],
    ) -> Result<Vec<Grant>, GrantError> {
        check_changes(object, changes)?;
        let insert = format!(
            "INSERT INTO {}.grants (subject_type, subject_id, privilege, object_type, object_id) \
             SELECT $1, $2, p, $4, $5 FROM unnest($3::text[]) AS p \
             ON CONFLICT DO NOTHING",
            self.schema
        );
        let delete = format!(
            "DELETE FROM {}.grants WHERE subject_type = $1 AND subject_id = $2 \
             AND privilege = ANY($3) AND object_type = $4 AND object_id = $5",
            self.schema
        );
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        for change in changes {
            for (sql, privileges) in [(&insert, &change.add), (&delete, &change.remove)] {
                if privileges.is_empty() {
                    continue;
                }
                sqlx::query(sql)
                    .bind(change.subject.kind.as_str())
                    .bind(&change.subject.id)
                    .bind(privileges)
                    .bind(&object.entity_type)
                    .bind(&object.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
            }
        }
        let rows: Vec<(String, String, String)> = sqlx::query_as(&self.list_query())
            .bind(&object.entity_type)
            .bind(&object.id)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        grants_of(object, rows)
    }

    async fn delete_object(
        &self,
        object: &ObjectRef,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError> {
        let sql = format!(
            "DELETE FROM {}.grants WHERE {}",
            self.schema,
            Self::AFFECTED
        );
        let result = sqlx::query(&sql)
            .bind(&object.entity_type)
            .bind(&object.id)
            .bind(descendant_types)
            .bind(&self.separator)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(result.rows_affected())
    }

    async fn rename(
        &self,
        object: &ObjectRef,
        new_id: &str,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError> {
        check_rename(object, new_id, &self.separator)?;
        // Copy to the new ids (keeping a grant the destination holds once), then drop the
        // old ones. The new ids are no descendants of the old one, so the delete can't
        // reach the copies.
        let copy = format!(
            "INSERT INTO {s}.grants (subject_type, subject_id, privilege, object_type, object_id) \
             SELECT subject_type, subject_id, privilege, object_type, \
                    $5 || substr(object_id, length($2) + 1) \
             FROM {s}.grants WHERE {affected} \
             ON CONFLICT DO NOTHING",
            s = self.schema,
            affected = Self::AFFECTED
        );
        let delete = format!(
            "DELETE FROM {}.grants WHERE {}",
            self.schema,
            Self::AFFECTED
        );
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        sqlx::query(&copy)
            .bind(&object.entity_type)
            .bind(&object.id)
            .bind(descendant_types)
            .bind(&self.separator)
            .bind(new_id)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let moved = sqlx::query(&delete)
            .bind(&object.entity_type)
            .bind(&object.id)
            .bind(descendant_types)
            .bind(&self.separator)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected();
        tx.commit().await.map_err(store_error)?;
        Ok(moved)
    }

    async fn groups(&self, subject: &Subject) -> Result<Vec<GroupMembership>, GrantError> {
        Ok(self.facts(subject, &[]).await?.groups)
    }

    async fn effective(
        &self,
        subject: &Subject,
        objects: &[ObjectRef],
    ) -> Result<Vec<Grant>, GrantError> {
        Ok(self.facts(subject, objects).await?.grants)
    }

    /// One query: the subject's groups (a recursive walk of `memberships`), and the grants
    /// it or one of them holds on `objects`.
    async fn facts(
        &self,
        subject: &Subject,
        objects: &[ObjectRef],
    ) -> Result<SubjectFacts, GrantError> {
        let s = &self.schema;
        let sql = format!(
            "WITH RECURSIVE member_of (group_id) AS ( \
                 SELECT group_id FROM {s}.memberships \
                 WHERE principal_type = $1 AND principal_id = $2 \
                 UNION \
                 SELECT m.group_id FROM {s}.memberships m \
                 JOIN member_of g ON m.principal_type = 'group' AND m.principal_id = g.group_id::text \
             ), \
             holders (subject_type, subject_id) AS ( \
                 SELECT $1::text, $2::text \
                 UNION ALL SELECT 'group', group_id::text FROM member_of \
             ) \
             SELECT 'group', g.id::text, g.name, \
                    CASE WHEN EXISTS (SELECT 1 FROM {s}.memberships d \
                                      WHERE d.group_id = g.id AND d.principal_type = $1 \
                                        AND d.principal_id = $2) \
                         THEN 'direct' ELSE '' END, \
                    NULL::text, NULL::text, \
                    ARRAY(SELECT p.name FROM {s}.memberships m JOIN {s}.groups p ON p.id = m.group_id \
                          WHERE m.principal_type = 'group' AND m.principal_id = g.id::text \
                          ORDER BY p.name) \
             FROM {s}.groups g WHERE g.id IN (SELECT group_id FROM member_of) \
             UNION ALL \
             SELECT 'grant', gr.subject_type, gr.subject_id, gr.privilege, gr.object_type, \
                    gr.object_id, NULL::text[] \
             FROM {s}.grants gr JOIN holders h \
               ON gr.subject_type = h.subject_type AND gr.subject_id = h.subject_id \
             WHERE (gr.object_type, gr.object_id) IN (SELECT * FROM unnest($3::text[], $4::text[]))"
        );
        type Row = (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<Vec<String>>,
        );
        let (types, ids): (Vec<&str>, Vec<&str>) = objects
            .iter()
            .map(|o| (o.entity_type.as_str(), o.id.as_str()))
            .unzip();
        let rows: Vec<Row> = sqlx::query_as(&sql)
            .bind(subject.kind.as_str())
            .bind(&subject.id)
            .bind(&types)
            .bind(&ids)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
        let mut facts = SubjectFacts::default();
        for (kind, a, b, c, object_type, object_id, parents) in rows {
            if kind == "group" {
                facts.groups.push(GroupMembership {
                    id: a,
                    name: b,
                    direct: c == "direct",
                    parents: parents.unwrap_or_default(),
                });
            } else if let (Some(object_type), Some(object_id)) = (object_type, object_id) {
                facts.grants.push(Grant {
                    subject: parse_subject(&a, b)?,
                    privilege: c,
                    object: ObjectRef::new(object_type, object_id),
                });
            }
        }
        facts.groups.sort_by(|a, b| a.name.cmp(&b.name));
        facts.grants.sort();
        Ok(facts)
    }
}
