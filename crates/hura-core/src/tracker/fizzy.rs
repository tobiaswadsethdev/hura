//! Fizzy: the cards assigned to you or created by you, on 37signals' hosted
//! Fizzy or on one you run yourself.
//!
//! **One token, possibly several accounts.** A Fizzy personal access token is
//! the person's, not an account's, and `/my/identity` lists every account it
//! can see with who the person is in each: a user id per account. So a
//! tracker reads all of them, or the one it names, and asks each for its
//! cards by that id.
//!
//! **Open cards only, by Fizzy's own rule.** Its card index leaves out closed
//! cards and those put off as *not now* unless it is asked for them, so
//! "mine and not done" is two filters, `assignee_ids[]` and `creator_ids[]`,
//! and no list of status names.
//!
//! A card's status is its column, which is somebody's name for a stage of
//! their process; one not yet triaged into a column is in Fizzy's *Maybe?*.
//! Fizzy has no pull requests, and its card list says nothing about comments,
//! so a Fizzy ticket is never announced for one.

use serde_json::Value;

use super::{
    Kind, Relation, Source, Task, answer, branch, raw, session_name, slug, string, urlencode,
};

/// Where 37signals runs it, for a tracker that names no site.
pub const HOSTED: &str = "https://app.fizzy.do";

/// What Fizzy calls the cards not yet triaged into a column.
const UNTRIAGED: &str = "Maybe?";

/// How many pages of one search are read. Fizzy's pages grow as they go
/// (15, 30, 50, then 100 at the time of writing), so this is a few hundred
/// cards: more than anybody has assigned to them, and less than a runaway
/// loop. The page size is Fizzy's to change, so the end is found by asking
/// for the page after it rather than by guessing from a page's length.
const MAX_PAGES: u32 = 5;

/// The site's address, without a trailing slash: the tracker's own, or the
/// hosted one. A bare host is taken to be HTTPS.
pub(super) fn site(source: &Source) -> String {
    let site = source
        .site
        .as_deref()
        .unwrap_or(HOSTED)
        .trim_end_matches('/');
    if site.contains("://") {
        site.to_string()
    } else {
        format!("https://{site}")
    }
}

fn auth(token: &str) -> String {
    format!("Bearer {token}")
}

/// One account the token can see, and who the token's owner is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Account {
    /// `/897362094`, as Fizzy writes it: the path every request starts with.
    slug: String,
    name: String,
    user: String,
    /// What goes in front of a card's number when the tracker reads more than
    /// one account (`acme#4`), since every account has a `#4` and the apps
    /// tell tickets apart by tracker and key. `None` with only one.
    label: Option<String>,
}

/// What one Fizzy tracker holds.
pub(super) struct Read {
    pub tasks: Vec<Task>,
    /// An account that could not be read, or was read only in part. The
    /// others still count.
    pub warnings: Vec<String>,
    /// Whether every card was read. One that was not is a ticket missing from
    /// the list without having gone anywhere, so a tracker read in part is
    /// not counted as read, and nothing is announced as having left it.
    pub complete: bool,
}

pub(super) fn read(source: &Source, token: &str, prefix: &str) -> Result<Read, String> {
    let site = site(source);
    let identity = get(&format!("{site}/my/identity"), token)?;
    let accounts = labelled(chosen(parse_identity(&identity)?, source.org.as_deref())?);

    let mut read = Read {
        tasks: Vec::new(),
        warnings: Vec::new(),
        complete: true,
    };
    let mut answered = 0;
    for account in &accounts {
        // Assigned first, so a card that is both is the one that asks
        // something of you. Without the cards you created, as a Jira
        // tracker's search leaves them out: `inbox` drops them either way,
        // and this saves asking.
        let mut searches = vec![("assignee_ids", Relation::Assigned)];
        if !source.hide_created {
            searches.push(("creator_ids", Relation::Created));
        }
        // One account failing is a warning about that account; the cards
        // the others hold are still yours.
        let mut found = Vec::new();
        let mut failed = None;
        for (filter, relation) in searches {
            let url = format!(
                "{site}{}/cards?{}={}",
                account.slug,
                urlencode(&format!("{filter}[]")),
                urlencode(&account.user)
            );
            match cards(&url, token) {
                Ok((cards, all)) => {
                    if !all {
                        read.complete = false;
                        read.warnings.push(format!(
                            "{}: more cards than are read here; the least active are left out",
                            account.name
                        ));
                    }
                    found.extend(cards.into_iter().map(|c| (c, relation)));
                }
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = failed {
            read.complete = false;
            read.warnings.push(format!("{}: {e}", account.name));
            continue;
        }
        answered += 1;
        for (card, relation) in found {
            let Some(task) = task(&card, source, &site, account, relation, prefix) else {
                continue;
            };
            if !read.tasks.iter().any(|t| t.id == task.id) {
                read.tasks.push(task);
            }
        }
    }
    if answered == 0 {
        // Nothing was read at all: one failure, said once, rather than a
        // tracker that answered with warnings.
        return Err(read.warnings.join("; "));
    }
    Ok(read)
}

/// Every page of one search, up to [`MAX_PAGES`], and whether that was all
/// of them.
fn cards(url: &str, token: &str) -> Result<(Vec<Value>, bool), String> {
    let mut all = Vec::new();
    for page in 1..=MAX_PAGES {
        let paged = if page == 1 {
            url.to_string()
        } else {
            format!("{url}&page={page}")
        };
        let (code, body) = raw(&paged, &auth(token), super::JSON, &[], None)?;
        // A page past the end is an answer, not a failure: a 404, or nothing.
        if page > 1 && code == 404 {
            return Ok((all, true));
        }
        let list = match answer(&String::from_utf8_lossy(&body), code)? {
            Value::Null => Vec::new(),
            Value::Array(list) => list,
            _ => return Err("fizzy did not answer with a list of cards".into()),
        };
        if list.is_empty() {
            return Ok((all, true));
        }
        all.extend(list);
    }
    Ok((all, false))
}

fn get(url: &str, token: &str) -> Result<Value, String> {
    super::get(url, &auth(token), &[])
}

fn parse_identity(body: &Value) -> Result<Vec<Account>, String> {
    let accounts = body
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or("fizzy did not say which accounts the token belongs to")?;
    Ok(accounts
        .iter()
        .filter_map(|a| {
            let slug = string(a, "slug");
            let user = a.get("user").map(|u| string(u, "id")).unwrap_or_default();
            if slug.is_empty() || user.is_empty() {
                return None;
            }
            // Written with its leading slash, which is what lets it go
            // straight after the site; said once more here in case it is not.
            let slug = format!("/{}", slug.trim_matches('/'));
            Some(Account {
                slug,
                name: string(a, "name"),
                user,
                label: None,
            })
        })
        .collect())
}

/// The account the tracker names, by name or by the number in its address,
/// or all of them.
fn chosen(accounts: Vec<Account>, named: Option<&str>) -> Result<Vec<Account>, String> {
    if accounts.is_empty() {
        return Err("the token belongs to no fizzy account".into());
    }
    let Some(named) = named else {
        return Ok(accounts);
    };
    let wanted = named.trim().trim_matches('/');
    let found: Vec<Account> = accounts
        .iter()
        .filter(|a| a.slug.trim_matches('/') == wanted || a.name.eq_ignore_ascii_case(wanted))
        .cloned()
        .collect();
    if found.is_empty() {
        let have: Vec<&str> = accounts.iter().map(|a| a.name.as_str()).collect();
        return Err(format!(
            "the token has no account `{wanted}`; it has {}",
            have.join(", ")
        ));
    }
    Ok(found)
}

/// Each account's [`Account::label`]: its name as a slug, or, where two
/// share one or one has none, the number in its address.
fn labelled(mut accounts: Vec<Account>) -> Vec<Account> {
    if accounts.len() < 2 {
        return accounts;
    }
    let names: Vec<String> = accounts.iter().map(|a| slug(&a.name)).collect();
    let distinct = names
        .iter()
        .enumerate()
        .all(|(i, n)| !n.is_empty() && !names[..i].contains(n));
    for (account, name) in accounts.iter_mut().zip(names) {
        account.label = Some(if distinct {
            name
        } else {
            account.slug.trim_matches('/').to_string()
        });
    }
    accounts
}

/// One card, as a task.
fn task(
    card: &Value,
    source: &Source,
    site: &str,
    account: &Account,
    relation: Relation,
    prefix: &str,
) -> Option<Task> {
    let number = card.get("number")?.as_u64()?;
    // A closed card is not asked for, but one closed between pages would be
    // a ticket that is done.
    if card.get("closed").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let title = string(card, "title");
    // `#4`, or `acme#4` beside other accounts, whose branch is `acme-4-…`:
    // a `#` is only dropped from the front of a key.
    let (key, branch_key) = match &account.label {
        Some(label) => (format!("{label}#{number}"), format!("{label}-{number}")),
        None => (format!("#{number}"), format!("#{number}")),
    };
    Some(Task {
        tracker: source.name.clone(),
        kind: Kind::Fizzy,
        // Numbers are per account, and a tracker can read several: the
        // account's slug keeps two `#4`s apart, and is where a card is
        // addressed anyway, `/{slug}/cards/{number}`.
        id: format!("{}/{number}", account.slug.trim_matches('/')),
        session_name: session_name(&branch_key, &title),
        branch: branch(prefix, &branch_key, &title),
        key,
        title,
        url: Some(string(card, "url"))
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| format!("{site}{}/cards/{number}", account.slug)),
        status: card
            .get("column")
            .map(|c| string(c, "name"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| UNTRIAGED.to_string()),
        // The board, which is the closest Fizzy has to a type, and the part
        // of a card's place that its column does not say.
        item_type: card
            .get("board")
            .map(|b| string(b, "name"))
            .unwrap_or_default(),
        repo: None,
        relation,
        filters: Vec::new(),
        updated: Some(string(card, "last_active_at")).filter(|s| !s.is_empty()),
        comments: None,
        last_commenter: None,
        last_comment_mine: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            kind: Kind::Fizzy,
            name: "fizzy".into(),
            ..Source::default()
        }
    }

    fn account() -> Account {
        Account {
            slug: "/897362094".into(),
            name: "37signals".into(),
            user: "03f5v9zjw7pz8717a4no1h8a7".into(),
            label: None,
        }
    }

    #[test]
    fn the_site_is_the_hosted_one_unless_it_says_otherwise() {
        assert_eq!(site(&source()), "https://app.fizzy.do");
        let mut own = source();
        own.site = Some("fizzy.example.com/".into());
        assert_eq!(site(&own), "https://fizzy.example.com");
        own.site = Some("https://cards.example.com".into());
        assert_eq!(site(&own), "https://cards.example.com");
    }

    #[test]
    fn the_identity_says_who_the_token_is_in_each_account() {
        let body = serde_json::json!({
            "id": "x",
            "accounts": [
                { "id": "a", "name": "37signals", "slug": "/897362094",
                  "user": { "id": "03f5v9zjw7pz8717a4no1h8a7", "name": "David" } },
                { "id": "b", "name": "Side project", "slug": "1234567",
                  "user": { "id": "u2" } },
                { "id": "c", "name": "No user", "slug": "/1" }
            ]
        });
        let accounts = parse_identity(&body).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0], account());
        assert_eq!(accounts[1].slug, "/1234567", "given its slash");
    }

    #[test]
    fn an_account_is_named_by_name_or_number() {
        let two = || {
            vec![
                account(),
                Account {
                    slug: "/1234567".into(),
                    name: "Side project".into(),
                    user: "u2".into(),
                    label: None,
                },
            ]
        };
        assert_eq!(chosen(two(), None).unwrap().len(), 2);
        assert_eq!(chosen(two(), Some("side PROJECT")).unwrap()[0].user, "u2");
        assert_eq!(
            chosen(two(), Some("/897362094")).unwrap()[0].name,
            "37signals"
        );
        let e = chosen(two(), Some("acme")).unwrap_err();
        assert!(e.contains("acme") && e.contains("Side project"), "{e}");
        assert!(chosen(Vec::new(), None).is_err());
    }

    #[test]
    fn a_card_is_a_task_in_its_column() {
        let card = serde_json::json!({
            "id": "03f5vaeq985jlvwv3arl4srq2",
            "number": 42,
            "title": "Retry uploads that time out",
            "status": "published",
            "closed": false,
            "postponed": false,
            "last_active_at": "2025-12-05T19:38:48.553Z",
            "url": "https://app.fizzy.do/897362094/cards/42",
            "board": { "id": "b", "name": "Website" },
            "column": { "id": "c", "name": "In Progress", "color": "var(--color-card-4)" },
            "creator": { "id": "someone-else" },
            "assignees": []
        });
        let t = task(
            &card,
            &source(),
            HOSTED,
            &account(),
            Relation::Assigned,
            "hura",
        )
        .unwrap();
        assert_eq!(t.kind, Kind::Fizzy);
        assert_eq!(t.id, "897362094/42");
        assert_eq!(t.key, "#42");
        assert_eq!(t.status, "In Progress");
        assert_eq!(t.item_type, "Website");
        assert_eq!(t.url, "https://app.fizzy.do/897362094/cards/42");
        assert_eq!(t.branch, "hura/42-retry-uploads-that-time-out");
        assert_eq!(t.updated.as_deref(), Some("2025-12-05T19:38:48.553Z"));
        assert_eq!(t.comments, None);
    }

    #[test]
    fn an_untriaged_card_is_a_maybe_and_a_closed_one_is_not_read() {
        let card = serde_json::json!({ "number": 7, "title": "Idea", "closed": false });
        let t = task(
            &card,
            &source(),
            HOSTED,
            &account(),
            Relation::Created,
            "hura",
        )
        .unwrap();
        assert_eq!(t.status, "Maybe?");
        assert_eq!(t.relation, Relation::Created);

        let closed = serde_json::json!({ "number": 8, "title": "Done", "closed": true });
        assert!(
            task(
                &closed,
                &source(),
                HOSTED,
                &account(),
                Relation::Created,
                "hura"
            )
            .is_none()
        );
    }

    #[test]
    fn beside_other_accounts_a_card_says_which_it_is_in() {
        let two = |b: &str| {
            labelled(vec![
                account(),
                Account {
                    slug: "/1234567".into(),
                    name: b.into(),
                    user: "u2".into(),
                    label: None,
                },
            ])
        };
        let named = two("Side Project");
        assert_eq!(named[0].label.as_deref(), Some("37signals"));
        assert_eq!(named[1].label.as_deref(), Some("side-project"));
        // Two of one name are their numbers instead.
        let same = two("37Signals");
        assert_eq!(same[0].label.as_deref(), Some("897362094"));
        assert_eq!(same[1].label.as_deref(), Some("1234567"));
        assert_eq!(labelled(vec![account()])[0].label, None, "alone, no label");

        let card = serde_json::json!({ "number": 4, "title": "Fix login" });
        let t = task(
            &card,
            &source(),
            HOSTED,
            &named[1],
            Relation::Assigned,
            "hura",
        )
        .unwrap();
        assert_eq!(t.key, "side-project#4");
        assert_eq!(t.branch, "hura/side-project-4-fix-login");
        assert_eq!(t.session_name, "side-project-4-fix-login");
    }

    #[test]
    fn a_card_with_no_url_is_given_the_one_it_is_at() {
        let card = serde_json::json!({ "number": 4, "title": "Fix login" });
        let t = task(
            &card,
            &source(),
            "https://fizzy.example.com",
            &account(),
            Relation::Assigned,
            "hura",
        )
        .unwrap();
        assert_eq!(t.url, "https://fizzy.example.com/897362094/cards/4");
    }
}
