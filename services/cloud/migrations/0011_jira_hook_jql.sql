-- The JQL a Jira webhook was made with. Jira refuses an empty filter, so it
-- names every project the connection could see; when that changes, the
-- webhook is made again, since a webhook's filter cannot be changed in place.
alter table jira_hooks add column jql text;
