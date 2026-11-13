-- Paying, through Stripe: a plan to pay for, and the account's customer and
-- subscription there as Stripe last said.

-- No limits, as free has none yet; what each has room for is set here.
insert into plans (id, max_trackers, max_filters, min_read_seconds)
values ('pro', null, null, 60)
on conflict (id) do nothing;

alter table accounts add column stripe_customer text unique;
-- `{id, status, current_period_end, cancel_at_period_end}`, or null.
alter table accounts add column subscription jsonb;
-- When the event the subscription was last set from was made, so an older
-- one arriving late does not undo a newer.
alter table accounts add column subscription_at bigint;
