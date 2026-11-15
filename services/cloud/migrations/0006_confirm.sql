-- A connection waiting to be confirmed: the tokens a provider gave, held
-- (encrypted with the account's key) until the person in the browser says
-- this is the hura account they meant, for the ten minutes a state lasts.
alter table oauth_states add column pending bytea;
