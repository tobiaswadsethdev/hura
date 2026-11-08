// hura cloud, from the settings screen: signing this computer in with an
// emailed code, and once in, the account, the devices signed in to it, and
// the trackers still kept on this computer.
//
// Optional, and never in the way: signed out, trackers are kept and read on
// this computer as they always were, and nothing here asks the service
// anything. Signed in, the trackers section below is the account's, the same
// on every device.

import { useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import type { CloudAccount } from "./gen/CloudAccount";
import type { CloudDevice } from "./gen/CloudDevice";
import type { CloudStatus } from "./gen/CloudStatus";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import { Forget } from "./icons";

export function AccountSection({
  onChanged,
}: {
  /// Signed in or out, or trackers moved: the trackers to show now, which
  /// are someone else's list than before.
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
      <h3>account</h3>
      {status === undefined && <Waiting />}
      {status === null && <SignIn onSignedIn={changed} />}
      {status && <SignedIn status={status} onSignedOut={() => changed(null)} onMoved={() => onChanged(true)} />}
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
        Sign in to hura cloud to keep your trackers and their filters in one place, the same on
        every device you sign in on. Optional: without an account, trackers stay on this computer,
        as they do now.
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

/// Who this computer is signed in as, its trackers not yet moved, and the
/// account's devices.
function SignedIn({
  status,
  onSignedOut,
  onMoved,
}: {
  status: CloudStatus;
  onSignedOut: () => void;
  onMoved: () => void;
}) {
  const [account, setAccount] = useState<CloudAccount | null>(null);
  const [devices, setDevices] = useState<CloudDevice[] | null>(null);
  const [local, setLocal] = useState<ConfiguredTracker[]>([]);
  // What stayed on this computer at the last move, because the account
  // already had a tracker by that name.
  const [kept, setKept] = useState<string[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Read through a ref: a new callback each render is not a reason to ask
  // the service again.
  const signedOut = useRef(onSignedOut);
  signedOut.current = onSignedOut;

  useEffect(() => {
    let live = true;
    Promise.all([api.cloudAccount(), api.cloudDevices(), api.localTrackers()]).then(
      ([a, d, l]) => {
        if (!live) return;
        setAccount(a);
        setDevices(d);
        setLocal(l);
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

      {local.length > 0 && (
        <div className="setting-group">
          <h4>on this computer only</h4>
          <p className="hint">
            {kept.length > 0
              ? `Your account already has a tracker called ${kept.join(", ")}, so this computer's stays here. Forget one of them to move it.`
              : `${local.map((t) => t.source.name).join(", ")}: added before you signed in, so not read while you are. Moving sends them and their tokens to hura cloud, which keeps the tokens encrypted, and this computer forgets them.`}
          </p>
          <div className="setting-actions">
            <button
              className="go"
              disabled={busy !== null}
              onClick={() =>
                void act("move", async () => {
                  const left = await api.moveTrackersToCloud();
                  setLocal(left);
                  setKept(left.map((t) => t.source.name));
                  onMoved();
                })
              }
            >
              {busy === "move" ? "moving…" : "move to my account"}
            </button>
          </div>
        </div>
      )}

      <h4>devices</h4>
      {devices === null && !error && <Waiting />}
      {devices?.map((d) => (
        <div key={d.id} className="row device">
          <span className="row-name">{d.name}</span>
          <span className="hint">{d.platform}</span>
          <span className="hint">{d.current ? "this computer" : `last seen ${ago(d.last_seen_at)}`}</span>
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
          Signing out goes back to the trackers kept on this computer. Your account keeps its own.
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
