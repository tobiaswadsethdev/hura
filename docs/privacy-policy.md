# hura privacy policy

*Last updated: 13 November 2026*

This policy is about **hura cloud**, the service behind an account in the hura
desktop and iPhone apps, and the **hura relay**, which delivers notifications
to iPhones. It says what is kept about you, why, who else handles it, and how
to take it away.

Using hura without an account (servers you run yourself, their sessions and
sandboxes) sends nothing to hura cloud: those stay on your own machines, and
this policy is not about them.

## Who is responsible

Tobias Wadseth, Sweden, is the controller of your personal
data. Write to **[privacy@gethura.app](mailto:privacy@gethura.app)** about
anything in this policy.

## What is kept, and why

**Your account.** The email address you sign in with, or, with Sign in with
Apple, the identifier Apple gives hura for your Apple ID and the address Apple
shares (which may be an address of Apple's that forwards to yours). When the
account was made, and which plan it is on. *Why:* to sign you in and to keep
your trackers yours (the contract between us, GDPR article 6(1)(b)).

**Sign-in codes.** A code emailed to you is stored only as a hash, used once,
and deleted after a day.

**Your devices.** Each phone or computer signed in: the name it gives itself
(often your name, as in "Anna's MacBook"), whether it is a Mac, a PC or an
iPhone, and when it was signed in and last used. Its sign-in tokens are stored
only as hashes. *Why:* so you can see and sign out your devices.

**Your trackers.** What you set up in hura: which GitHub, Jira, Azure DevOps or
Fizzy sites and projects to read, your filters, and the access token or
connection that reads them. Tokens are **encrypted** with a key of your
account's own and are never shown back, to you or anyone. For a GitHub
connection, your GitHub username is kept; for Jira, the site's address. *Why:*
to read your tickets and pull requests for you (contract).

**What was last seen on them.** To tell you what changed, hura keeps, per
ticket and pull request that is yours: its key (as in `PROJ-7`), its status,
how many comments it has, and its review and check state. Not titles,
descriptions or comments. *Why:* to notify you once about each change
(contract).

**Notifications.** For an iPhone that has turned them on: the address of its
link on the hura relay and the keys to it. The text of a notification is
encrypted on hura cloud and can be read only by your phone. *Why:* to notify
you (contract).

**Payments.** If you subscribe: your customer and subscription identifiers at
Stripe, the subscription's status and when it renews. Card details go to
Stripe only and never reach hura. *Why:* to provide the plan you pay for
(contract), and to keep records the law requires (legal obligation, 6(1)(c)).

**Your address on the network.** Each request's IP address is used, in memory
only, to slow down anyone asking too often; it is not written down. The relay
keeps an IP address for one hour when a phone registers, for the same reason.
*Why:* to keep the service safe and working (legitimate interest, 6(1)(f)).

**Server logs.** Errors and the service's own activity are logged with your
account's identifier (a random ID), never your email or tokens, and kept for
30 days. *Why:* to find and fix problems (legitimate interest).

hura has **no analytics, no advertising and no tracking**, and sells nothing
about you to anybody.

## Who else handles it

hura cloud, its database and its backups run on hura's own server, in the EU,
and are not handed to a hosting company. Beyond that, only to provide the
service, under their own data processing terms:

| Who | For | Where |
| --- | --- | --- |
| Cloudflare | the network in front of hura cloud, and running the relay | worldwide, EU–US Data Privacy Framework |
| Resend | sending sign-in codes by email | USA, EU–US Data Privacy Framework / standard contractual clauses |
| Apple | Sign in with Apple, and delivering notifications to iPhones | worldwide |
| Stripe | payments, if you subscribe | EU and USA, EU–US Data Privacy Framework |

hura also talks to **GitHub, Atlassian (Jira), Microsoft (Azure DevOps) and
Fizzy** on your behalf, with the access you gave it, to read and change your
tickets and pull requests as you ask. What they do with that is up to their own
policies and your agreement with them.

Where data leaves the EU/EEA, it does so under the EU–US Data Privacy Framework
or the European Commission's standard contractual clauses.

## How long it is kept

As long as you have an account. When you delete it, everything above is deleted
at once, except: database backups, on hura's own server, which roll over
within 30 days; records of
payments, which Stripe and hura must keep for as long as accounting law
requires (7 years in Sweden); and logs, within 30 days.

## Your rights

- **See and take it with you.** *Export* in the account screen of either app
  downloads everything hura cloud keeps about you, as JSON (tokens are said to
  be there, never shown).
- **Delete it.** *Delete account* in the account screen of either app deletes
  the account and everything in it at once, cancels a subscription, and signs
  out every device.
- **Correct it**, **restrict** or **object to** how it is used: write to
  [privacy@gethura.app](mailto:privacy@gethura.app).
- **Complain** to the Swedish Authority for Privacy Protection
  (Integritetsskyddsmyndigheten, [imy.se](https://www.imy.se)), or the
  authority where you live.

## Children

hura is not meant for anyone under 16, and does not knowingly keep anything
about them.

## Changes

A change to this policy is published here with a new date. A change that
matters, such as a new kind of data or a new party it goes to, is also told
in the apps or by email before it takes effect.
