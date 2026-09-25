import type { CapabilityRequest, CapabilityResponse } from "./capabilities.js";
import type { ExtensionView, FormValues } from "./models.js";

/** Strict UTF-8 JSON Lines: one exact message object per line; reject malformed JSON, unknown fields, mismatched variants, version mismatches, and over-limit lines. */
export const HOST_WORKER_PROTOCOL_VERSION = 1 as const;

/** Limits are part of the wire contract; the host enforces UTF-8 byte and item counts. */
export const PROTOCOL_LIMITS = {
  jsonLineBytes: 262_144,
  requestIdCharacters: 64,
  activeRequestsPerCommand: 32,
  formValueFields: 32,
  formValueTextCharacters: 4_096,
  capabilityTextCharacters: 16_384,
  capabilityResponseBytes: 65_536,
} as const;

export type WorkerToHostMessage =
  | {
      readonly protocolVersion: typeof HOST_WORKER_PROTOCOL_VERSION;
      readonly kind: "view.update";
      readonly commandId: string;
      readonly view: ExtensionView;
    }
  | {
      readonly protocolVersion: typeof HOST_WORKER_PROTOCOL_VERSION;
      readonly kind: "capability.request";
      readonly commandId: string;
      readonly requestId: string;
      readonly request: CapabilityRequest;
    }
  | {
      readonly protocolVersion: typeof HOST_WORKER_PROTOCOL_VERSION;
      readonly kind: "command.complete";
      readonly commandId: string;
    };

export type HostToWorkerMessage =
  | {
      readonly protocolVersion: typeof HOST_WORKER_PROTOCOL_VERSION;
      readonly kind: "command.invoke";
      readonly commandId: string;
      readonly input: FormValues;
    }
  | {
      readonly protocolVersion: typeof HOST_WORKER_PROTOCOL_VERSION;
      readonly kind: "action.invoke";
      readonly commandId: string;
      readonly actionId: string;
      readonly itemId?: string;
      readonly values: FormValues;
    }
  | {
      readonly protocolVersion: typeof HOST_WORKER_PROTOCOL_VERSION;
      readonly kind: "command.cancel";
      readonly commandId: string;
    }
  | {
      readonly protocolVersion: typeof HOST_WORKER_PROTOCOL_VERSION;
      readonly kind: "capability.response";
      readonly requestId: string;
      readonly response: CapabilityResponse;
    };
