import type { Client } from "@urql/core";
import { backendClient } from "@/lib/graphql/urql-client";
import {
  webauthnAuthenticateCompleteMutation,
  webauthnAuthenticateStartMutation,
  webauthnRegisterCompleteMutation,
  webauthnRegisterStartMutation,
} from "@/lib/graphql/mutations";
import type { AuthUser } from "@/lib/hooks/use-auth";
import type { PasskeySummary } from "@/lib/types/settings";

type JsonCreationOptions = {
  challenge: string;
  rp: PublicKeyCredentialRpEntity;
  user: PublicKeyCredentialUserEntity & { id: string };
  pubKeyCredParams: PublicKeyCredentialParameters[];
  timeout?: number;
  excludeCredentials?: Array<PublicKeyCredentialDescriptor & { id: string }>;
  authenticatorSelection?: AuthenticatorSelectionCriteria;
  attestation?: AttestationConveyancePreference;
  extensions?: AuthenticationExtensionsClientInputs;
};

type JsonRequestOptions = {
  challenge: string;
  timeout?: number;
  rpId?: string;
  allowCredentials?: Array<PublicKeyCredentialDescriptor & { id: string }>;
  userVerification?: UserVerificationRequirement;
  extensions?: AuthenticationExtensionsClientInputs;
};

type LoginPayload = {
  token: string;
  user: AuthUser | null;
};

type PublicKeyCredentialJsonHelpers = {
  parseCreationOptionsFromJSON?: (value: unknown) => PublicKeyCredentialCreationOptions;
  parseRequestOptionsFromJSON?: (value: unknown) => PublicKeyCredentialRequestOptions;
};

export class PasskeyClientError extends Error {
  readonly code: "unsupported" | "cancelled" | "invalid_response" | "failed";

  constructor(code: "unsupported" | "cancelled" | "invalid_response" | "failed", message: string) {
    super(message);
    this.code = code;
  }
}

function credentialHelpers(): PublicKeyCredentialJsonHelpers {
  return PublicKeyCredential as unknown as PublicKeyCredentialJsonHelpers;
}

function ensurePasskeySupport() {
  if (
    typeof window === "undefined" ||
    typeof PublicKeyCredential === "undefined" ||
    typeof navigator === "undefined" ||
    typeof navigator.credentials?.create !== "function" ||
    typeof navigator.credentials?.get !== "function"
  ) {
    throw new PasskeyClientError("unsupported", "Passkeys are not supported in this browser.");
  }
}

function toUint8Array(value: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }

  return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
}

function base64UrlToBuffer(value: string): ArrayBuffer {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
  const binary = window.atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

function bufferToBase64Url(value: ArrayBuffer | ArrayBufferView | null | undefined): string | null {
  if (!value) {
    return null;
  }

  const bytes = toUint8Array(value);
  let binary = "";
  bytes.forEach((byte) => {
    binary += String.fromCharCode(byte);
  });

  return window
    .btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

function parseCreationOptions(optionsJson: string): PublicKeyCredentialCreationOptions {
  const parsed = JSON.parse(optionsJson) as JsonCreationOptions;
  const helper = credentialHelpers();
  if (typeof helper.parseCreationOptionsFromJSON === "function") {
    return helper.parseCreationOptionsFromJSON(parsed);
  }

  return {
    ...parsed,
    challenge: base64UrlToBuffer(parsed.challenge),
    user: {
      ...parsed.user,
      id: base64UrlToBuffer(parsed.user.id),
    },
    excludeCredentials: parsed.excludeCredentials?.map((credential) => ({
      ...credential,
      id: base64UrlToBuffer(credential.id),
    })),
  };
}

function parseRequestOptions(optionsJson: string): PublicKeyCredentialRequestOptions {
  const parsed = JSON.parse(optionsJson) as JsonRequestOptions;
  const helper = credentialHelpers();
  if (typeof helper.parseRequestOptionsFromJSON === "function") {
    return helper.parseRequestOptionsFromJSON(parsed);
  }

  return {
    ...parsed,
    challenge: base64UrlToBuffer(parsed.challenge),
    allowCredentials: parsed.allowCredentials?.map((credential) => ({
      ...credential,
      id: base64UrlToBuffer(credential.id),
    })),
  };
}

function credentialToJson(credential: PublicKeyCredential): string {
  const jsonValue = (credential as PublicKeyCredential & { toJSON?: () => unknown }).toJSON?.();
  if (jsonValue) {
    return JSON.stringify(jsonValue);
  }

  const base = {
    id: credential.id,
    type: credential.type,
    rawId: bufferToBase64Url(credential.rawId),
    authenticatorAttachment: credential.authenticatorAttachment ?? undefined,
    clientExtensionResults: credential.getClientExtensionResults(),
  };

  const response = credential.response;
  if ("attestationObject" in response) {
    const attestation = response as AuthenticatorAttestationResponse;
    return JSON.stringify({
      ...base,
      response: {
        clientDataJSON: bufferToBase64Url(attestation.clientDataJSON),
        attestationObject: bufferToBase64Url(attestation.attestationObject),
        transports:
          typeof attestation.getTransports === "function"
            ? attestation.getTransports()
            : undefined,
      },
    });
  }

  const assertion = response as AuthenticatorAssertionResponse;
  return JSON.stringify({
    ...base,
    response: {
      clientDataJSON: bufferToBase64Url(assertion.clientDataJSON),
      authenticatorData: bufferToBase64Url(assertion.authenticatorData),
      signature: bufferToBase64Url(assertion.signature),
      userHandle: bufferToBase64Url(assertion.userHandle),
    },
  });
}

async function runMutation<TData, TVariables extends object>(
  client: Client,
  mutation: string,
  variables: TVariables,
  field: keyof TData,
): Promise<TData[keyof TData]> {
  const result = await client.mutation<TData, TVariables>(mutation, variables).toPromise();
  if (result.error || !result.data?.[field]) {
    throw result.error ?? new PasskeyClientError("failed", "Passkey request failed.");
  }

  return result.data[field];
}

function normalizePasskeyError(error: unknown): never {
  if (error instanceof PasskeyClientError) {
    throw error;
  }

  if (error instanceof DOMException && error.name === "NotAllowedError") {
    throw new PasskeyClientError("cancelled", "Passkey request was cancelled.");
  }

  if (error instanceof Error) {
    throw new PasskeyClientError("failed", error.message);
  }

  throw new PasskeyClientError("failed", "Passkey request failed.");
}

export function passkeysSupported(): boolean {
  try {
    ensurePasskeySupport();
    return true;
  } catch {
    return false;
  }
}

export async function authenticateWithPasskey(
  username?: string,
  client: Client = backendClient,
): Promise<LoginPayload> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        webauthnAuthenticateStart: {
          challengeId: string;
          optionsJson: string;
        };
      },
      { username?: string | null }
    >(
      client,
      webauthnAuthenticateStartMutation,
      { username: username?.trim() ? username.trim() : null },
      "webauthnAuthenticateStart",
    );

    const credential = await navigator.credentials.get({
      publicKey: parseRequestOptions(start.optionsJson),
    });
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("invalid_response", "No passkey assertion was returned.");
    }

    return runMutation<
      { webauthnAuthenticateComplete: LoginPayload },
      { input: { challengeId: string; responseJson: string } }
    >(
      client,
      webauthnAuthenticateCompleteMutation,
      {
        input: {
          challengeId: start.challengeId,
          responseJson: credentialToJson(credential),
        },
      },
      "webauthnAuthenticateComplete",
    ) as Promise<LoginPayload>;
  } catch (error) {
    normalizePasskeyError(error);
  }
}

export async function registerPasskey(client: Client = backendClient): Promise<PasskeySummary> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        webauthnRegisterStart: {
          challengeId: string;
          optionsJson: string;
        };
      },
      Record<string, never>
    >(client, webauthnRegisterStartMutation, {}, "webauthnRegisterStart");

    const credential = await navigator.credentials.create({
      publicKey: parseCreationOptions(start.optionsJson),
    });
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("invalid_response", "No passkey registration was returned.");
    }

    return runMutation<
      { webauthnRegisterComplete: PasskeySummary },
      {
        input: {
          challengeId: string;
          responseJson: string;
          friendlyName: string | null;
        };
      }
    >(
      client,
      webauthnRegisterCompleteMutation,
      {
        input: {
          challengeId: start.challengeId,
          responseJson: credentialToJson(credential),
          friendlyName: null,
        },
      },
      "webauthnRegisterComplete",
    ) as Promise<PasskeySummary>;
  } catch (error) {
    normalizePasskeyError(error);
  }
}
