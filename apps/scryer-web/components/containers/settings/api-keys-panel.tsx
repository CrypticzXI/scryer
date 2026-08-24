import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { createMyApiKeyMutation, revokeMyApiKeyMutation } from "@/lib/graphql/mutations";
import { myApiKeysQuery } from "@/lib/graphql/queries";
import type { ApiKeySummary } from "@/lib/types/settings";

type MyApiKeysQueryResult = {
  canCreateMyApiKeys: boolean;
  myApiKeys: ApiKeySummary[];
};

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

function apiKeyStatus(key: ApiKeySummary, canCreate: boolean) {
  if (key.revokedAt) {
    return "revoked";
  }
  if (key.expiresAt && new Date(key.expiresAt).getTime() <= Date.now()) {
    return "expired";
  }
  if (!canCreate) {
    return "disabled by current security policy";
  }
  return "active";
}

export function ApiKeysPanel() {
  const client = useClient();
  const [keys, setKeys] = useState<ApiKeySummary[]>([]);
  const [canCreate, setCanCreate] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [label, setLabel] = useState("");
  const [expiry, setExpiry] = useState("DAYS_90");
  const [revealed, setRevealed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revokingId, setRevokingId] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const load = useCallback(async () => {
    const result = await client
      .query<MyApiKeysQueryResult>(myApiKeysQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (result.error || !result.data) {
      setStatus(errorMessage(result.error, "Unable to load API keys."));
      return;
    }
    setKeys(result.data.myApiKeys);
    setCanCreate(result.data.canCreateMyApiKeys);
    setLoaded(true);
    setStatus(null);
  }, [client]);

  useEffect(() => {
    void load();
  }, [load]);

  const create = useCallback(async () => {
    const trimmedLabel = label.trim();
    if (!trimmedLabel) {
      setStatus("Enter a name for this API key.");
      return;
    }
    setBusy(true);
    setStatus(null);
    try {
      const result = await client
        .mutation<{ createMyApiKey?: { apiKey: string; key: ApiKeySummary } }>(
          createMyApiKeyMutation,
          { input: { label: trimmedLabel, expiry } },
        )
        .toPromise();
      const created = result.data?.createMyApiKey;
      if (result.error || !created) {
        setStatus(errorMessage(result.error, "Unable to create API key."));
        return;
      }
      setRevealed(created.apiKey);
      setLabel("");
      await load();
    } finally {
      setBusy(false);
    }
  }, [client, expiry, label, load]);

  const copyRevealedKey = useCallback(async () => {
    if (!revealed) {
      return;
    }
    try {
      if (!navigator.clipboard) {
        throw new Error("Clipboard access is unavailable.");
      }
      await navigator.clipboard.writeText(revealed);
      setStatus("API key copied.");
    } catch (error) {
      setStatus(errorMessage(error, "Unable to copy the API key."));
    }
  }, [revealed]);

  const revoke = useCallback(async (id: string) => {
    if (!window.confirm("Revoke this API key? Existing integrations will stop immediately.")) {
      return;
    }
    setRevokingId(id);
    setStatus(null);
    try {
      const result = await client.mutation(revokeMyApiKeyMutation, { id }).toPromise();
      if (result.error || !result.data?.revokeMyApiKey?.revoked) {
        setStatus(errorMessage(result.error, "Unable to revoke API key."));
        return;
      }
      await load();
    } finally {
      setRevokingId(null);
    }
  }, [client, load]);

  return (
    <section className="mt-6 space-y-3 rounded border border-[var(--scry-border)] p-4">
      <div>
        <h2 className="text-lg font-semibold">API keys</h2>
        <p className="text-sm text-[var(--scry-muted3)]">
          Keys act as <code>api (&lt;name&gt;) obo &lt;you&gt;</code>, are HTTP-only, and cannot complete MFA step-up.
        </p>
      </div>

      {status ? <p role="alert" className="text-sm text-[var(--scry-muted2)]">{status}</p> : null}

      {revealed ? (
        <div className="space-y-2 rounded bg-[var(--scry-panel)] p-3">
          <p className="text-sm">Copy this key now. It will not be shown again.</p>
          <code className="block break-all">{revealed}</code>
          <div className="flex gap-2">
            <button type="button" onClick={() => void copyRevealedKey()}>Copy key</button>
            <button type="button" onClick={() => setRevealed(null)}>Dismiss</button>
          </div>
        </div>
      ) : null}

      {canCreate ? (
        <div className="flex flex-wrap gap-2">
          <input
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            maxLength={120}
            placeholder="API key name"
            disabled={busy}
          />
          <select value={expiry} onChange={(event) => setExpiry(event.target.value)} disabled={busy}>
            <option value="DAYS_30">30 days</option>
            <option value="DAYS_90">90 days</option>
            <option value="DAYS_365">1 year</option>
            <option value="NEVER">Never expires</option>
          </select>
          <button type="button" disabled={busy} onClick={() => void create()}>Create API key</button>
        </div>
      ) : loaded ? (
        <p className="text-sm text-[var(--scry-muted3)]">
          API key creation is disabled by the current security policy.
        </p>
      ) : null}

      <div className="space-y-2">
        {keys.map((key) => (
          <div key={key.id} className="flex flex-wrap items-center justify-between gap-2 rounded bg-[var(--scry-panel)] p-3">
            <div>
              <div>{key.label} {key.provisioningSource === "environment" ? "(managed)" : ""}</div>
              <div className="text-sm text-[var(--scry-muted3)]">
                {key.actor} · created {key.createdAt} · expires{" "}
                {key.expiresAt ?? "never"} · last used {key.lastUsedAt ?? "never"} · status{" "}
                {apiKeyStatus(key, canCreate)}
              </div>
            </div>
            {key.provisioningSource === "user" && !key.revokedAt ? (
              <button type="button" disabled={revokingId === key.id} onClick={() => void revoke(key.id)}>
                {revokingId === key.id ? "Revoking…" : "Revoke"}
              </button>
            ) : null}
          </div>
        ))}
      </div>
    </section>
  );
}
