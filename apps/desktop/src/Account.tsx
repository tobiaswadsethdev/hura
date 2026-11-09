// hura cloud, on a screen of its own: signing this computer in with an
// emailed code, and once in, the account and the devices signed in to it.
//
// Its own destination rather than a section of the settings screen, as it is
// its own tab on the phone: settings are answers about this window and this
// server, and an account is somewhere you go to sign in or out.
//
// Trackers need it: they are kept and read on the account, the same on every
// device, and the trackers section of the settings screen is empty until this
// computer signs in. Everything else in hura works signed out.

import { useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import type { CloudAccount } from "./gen/CloudAccount";
import type { CloudDevice } from "./gen/CloudDevice";
import type { CloudStatus } from "./gen/CloudStatus";
import { Account as AccountGlyph, Forget } from "./icons";
import { Screen } from "./Screen";

export function AccountScreen({
  onChanged,
  onClose,
}: {
  /// Signed in or out: whether there are trackers to show now.
  onChanged: (signedIn: boolean) => void;
  onClose: () => void;
}) {
  return (
    <Screen icon={AccountGlyph} title="account" narrow onClose={onClose}>
      <AccountSection onChanged={onChanged} />
    </Screen>
  );
}

function AccountSection({
  onChanged,
}: {
  /// Signed in or out: whether there are trackers to show now.
  onChanged: (signedIn: boolean) => void;
}) {
  // `undefined` until the session file has been read.
  const [status, setStatus] = useState<CloudStatus | null | undefined>(undefined);

  useEffect(() => {
    api.cloudStatus().then(setStatus, () => setStatus(null));
  }, []);

  const changed = (next: CloudStatus | null) => {
    setStatus(next);
    onChanged(next !== null);
  };

  return (
    <section className="setting-group panel account">
      {/* What the account is, under a screen already titled "account". */}
      <h3>hura cloud</h3>
      {status === undefined && <Waiting />}
      {status === null && <SignIn onSignedIn={changed} />}
      {status && <SignedIn status={status} onSignedOut={() => changed(null)} />}
    </section>
  );
}

/// An address, then the code emailed to it.
function SignIn({ onSignedIn }: { onSignedIn: (status: CloudStatus) => void }) {
  const [email, setEmail] = useState("");
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  };

  const send = (to: string) =>
    run(async () => {
      await api.cloudSendCode(to);
      setSentTo(to.trim());
    });

  const digits = code.replace(/\D/g, "");

  return (
    <>
      <p className="hint">
        Sign in to hura cloud to use trackers: your tickets and pull requests, kept in one place
        and the same on every device you sign in on, with notifications when they change.
      </p>
      {sentTo === null ? (
        <>
          <label>
            <span>email</span>
            <input
              type="email"
              value={email}
              placeholder="you@example.com"
              onChange={(e) => setEmail(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && email.includes("@")) void send(email);
              }}
            />
          </label>
          <div className="setting-actions">
            <button className="go" disabled={busy || !email.includes("@")} onClick={() => void send(email)}>
              {busy ? "sending…" : "email me a code"}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="hint">
            A code was sent to <strong>{sentTo}</strong>. It works for ten minutes, once.
          </p>
          <label>
            <span>code</span>
            <input
              value={code}
              placeholder="six digits"
              inputMode="numeric"
              autoComplete="one-time-code"
              autoFocus
              onChange={(e) => setCode(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && digits.length === 6) {
                  void run(async () => onSignedIn(await api.cloudVerify(sentTo, digits)));
                }
              }}
            />
          </label>
          <div className="setting-actions">
            <button className="quiet" disabled={busy} onClick={() => void send(sentTo)}>
              send another
            </button>
            <button
              className="quiet"
              disabled={busy}
              onClick={() => {
                setSentTo(null);
                setCode("");
                setError(null);
              }}
            >
              use another address
            </button>
            <button
              className="go"
              disabled={busy || digits.length !== 6}
              onClick={() => void run(async () => onSignedIn(await api.cloudVerify(sentTo, digits)))}
            >
              {busy ? "signing in…" : "sign in"}
            </button>
          </div>
        </>
      )}
      {error && <p className="error">{error}</p>}
    </>
  );
}

/// Who this computer is signed in as, and the account's devices.
function SignedIn({
  status,
  onSignedOut,
}: {
  status: CloudStatus;
  onSignedOut: () => void;
}) {
  const [account, setAccount] = useState<CloudAccount | null>(null);
  const [devices, setDevices] = useState<CloudDevice[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Read through a ref: a new callback each render is not a reason to ask
  // the service again.
  const signedOut = useRef(onSignedOut);
  signedOut.current = onSignedOut;

  useEffect(() => {
    let live = true;
    Promise.all([api.cloudAccount(), api.cloudDevices()]).then(
      ([a, d]) => {
        if (!live) return;
        setAccount(a);
        setDevices(d);
      },
      async (e) => {
        if (!live) return;
        setError(messageOf(e));
        // Signed out from another device: the session is gone here too.
        if ((await api.cloudStatus()) === null) signedOut.current();
      },
    );
    return () => {
      live = false;
    };
  }, []);

  const act = async (what: string, work: () => Promise<void>) => {
    setBusy(what);
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <label>
        <span>email</span>
        <span className="value">{status.email}</span>
      </label>
      {account && (
        <label>
          <span>plan</span>
          <span className="value">{account.plan}</span>
        </label>
      )}

      <h4>devices</h4>
      {devices === null && !error && <Waiting />}
      {devices?.map((d) => (
        // Two lines, as a device is listed under an Apple Account: the name,
        // and under it what it is and when it was last seen. One line was four
        // columns, and no width for them lined up whatever the names were.
        <div key={d.id} className="row device">
          <span className="device-text">
            <span className="row-name">{d.name}</span>
            <span className="hint">
              {d.platform} · {d.current ? "this computer" : `last seen ${ago(d.last_seen_at)}`}
            </span>
          </span>
          <span className="row-actions">
            {!d.current && (
              <button
                className="quiet-icon danger"
                disabled={busy !== null}
                title={`sign ${d.name} out`}
                onClick={() =>
                  void act(`device:${d.id}`, async () => setDevices(await api.cloudSignOutDevice(d.id)))
                }
              >
                <Forget aria-label={`sign ${d.name} out`} />
              </button>
            )}
          </span>
        </div>
      ))}

      <div className="setting-actions">
        <span className="hint">
          Signed out, this computer has no trackers. Your account keeps them for when you sign in again.
        </span>
        <button
          className="quiet danger"
          disabled={busy !== null}
          onClick={() =>
            void act("sign-out", async () => {
              await api.cloudSignOut();
              onSignedOut();
            })
          }
        >
          {busy === "sign-out" ? "signing out…" : "sign out"}
        </button>
      </div>
      {error && <p className="error">{error}</p>}
    </>
  );
}

/// How long ago, in the largest unit that fits.
function ago(seconds: number): string {
  const s = Math.max(0, Date.now() / 1000 - seconds);
  if (s < 90) return "just now";
  if (s < 90 * 60) return `${Math.round(s / 60)} minutes ago`;
  if (s < 36 * 3600) return `${Math.round(s / 3600)} hours ago`;
  return `${Math.round(s / 86400)} days ago`;
}
