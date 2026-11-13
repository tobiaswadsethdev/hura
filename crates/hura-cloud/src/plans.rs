//! What an account's plan has room for: how many trackers, and how many
//! filters across them. Null in `plans` is no limit, which is what `free`
//! has until there is a plan to pay for.
//!
//! **Checked around a change**, inside its transaction: the account's use
//! before and after, and the change refused only when it grows past the
//! limit. An account already past one (a plan made smaller, or changed
//! down) can still edit and take away; it cannot add.

use axum::http::StatusCode;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    trackers: i64,
    filters: i64,
    max_trackers: Option<i32>,
    max_filters: Option<i32>,
}

pub async fn usage(db: &mut PgConnection, account: Uuid) -> Result<Usage> {
    let (trackers, filters, max_trackers, max_filters): (i64, i64, Option<i32>, Option<i32>) =
        sqlx::query_as(
            "select (select count(*) from trackers where account_id = $1),
                    (select coalesce(sum(jsonb_array_length(
                        coalesce(source->'jql_filters', '[]'::jsonb))), 0)::bigint
                     from trackers where account_id = $1),
                    p.max_trackers, p.max_filters
             from accounts a join plans p on p.id = a.plan where a.id = $1",
        )
        .bind(account)
        .fetch_one(&mut *db)
        .await?;
    Ok(Usage {
        trackers,
        filters,
        max_trackers,
        max_filters,
    })
}

/// Whether going from `before` to `after` is within the plan, or why not.
pub fn allowed(before: &Usage, after: &Usage) -> Result<()> {
    let over =
        |now: i64, was: i64, max: Option<i32>| max.filter(|&m| now > was && now > i64::from(m));
    if let Some(max) = over(after.trackers, before.trackers, after.max_trackers) {
        return Err(Error::new(
            StatusCode::PAYMENT_REQUIRED,
            format!("your plan has room for {max} tracker{}", plural(max)),
        ));
    }
    if let Some(max) = over(after.filters, before.filters, after.max_filters) {
        return Err(Error::new(
            StatusCode::PAYMENT_REQUIRED,
            format!("your plan has room for {max} filter{}", plural(max)),
        ));
    }
    Ok(())
}

fn plural(n: i32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(trackers: i64, filters: i64) -> Usage {
        Usage {
            trackers,
            filters,
            max_trackers: Some(2),
            max_filters: Some(3),
        }
    }

    #[test]
    fn only_growing_past_a_limit_is_refused() {
        assert!(allowed(&usage(1, 0), &usage(2, 3)).is_ok());
        assert!(allowed(&usage(2, 0), &usage(3, 0)).is_err());
        assert!(allowed(&usage(2, 3), &usage(2, 4)).is_err());
        // Already past it: editing and taking away still work.
        assert!(allowed(&usage(5, 9), &usage(5, 9)).is_ok());
        assert!(allowed(&usage(5, 9), &usage(4, 7)).is_ok());
        let none = Usage {
            max_trackers: None,
            max_filters: None,
            ..usage(0, 0)
        };
        assert!(
            allowed(
                &none,
                &Usage {
                    trackers: 99,
                    filters: 99,
                    ..none
                }
            )
            .is_ok()
        );
    }
}
