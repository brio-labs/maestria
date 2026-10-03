/** Explicit capability names supported by the initial Sillage SDK. */
export type CapabilityKind =
  | "fileSearch"
  | "userFileRead"
  | "http"
  | "storage"
  | "notification"
  | "open"
  | "copy";

export type ManifestPermission =
  | { readonly type: "fileSearch"; readonly maxResults?: number }
  | { readonly type: "userFileRead" }
  | { readonly type: "http"; readonly origins: readonly string[] }
  | { readonly type: "storage"; readonly scope: "extension" }
  | { readonly type: "notification" }
  | { readonly type: "open"; readonly targets: readonly ("url" | "selectedFile")[] }
  | { readonly type: "copy"; readonly formats: readonly "text"[] };

/** Opaque IDs and bounded strings are validated and authorized by the host. */
export type CapabilityRequest =
  | { readonly capability: "fileSearch"; readonly query: string; readonly limit: number }
  | {
      readonly capability: "userFileRead";
      readonly selectionId: string;
      readonly maxBytes: number;
    }
  | {
      readonly capability: "http";
      readonly url: string;
      readonly method: "GET" | "POST";
      readonly body?: string;
    }
  | {
      readonly capability: "storage";
      readonly operation: "get";
      readonly key: string;
    }
  | {
      readonly capability: "storage";
      readonly operation: "set";
      readonly key: string;
      readonly value: string;
    }
  | {
      readonly capability: "storage";
      readonly operation: "delete";
      readonly key: string;
    }
  | {
      readonly capability: "notification";
      readonly title: string;
      readonly message: string;
    }
  | {
      readonly capability: "open";
      readonly target:
        | { readonly kind: "url"; readonly url: string }
        | { readonly kind: "selectedFile"; readonly selectionId: string };
    }
  | { readonly capability: "copy"; readonly text: string };

export interface FileSearchResult {
  /** Opaque, host-issued reference; never a path. */
  readonly fileId: string;
  readonly title: string;
  readonly snippet?: string;
}

export type CapabilityFailureCode =
  | "permission_denied"
  | "cancelled"
  | "not_found"
  | "invalid_request"
  | "unavailable"
  | "failed";

export interface CapabilityFailure {
  readonly ok: false;
  readonly capability: CapabilityKind;
  readonly error: {
    readonly code: CapabilityFailureCode;
    readonly message: string;
  };
}

export type CapabilityResponse =
  | {
      readonly ok: true;
      readonly capability: "fileSearch";
      readonly results: readonly FileSearchResult[];
    }
  | { readonly ok: true; readonly capability: "userFileRead"; readonly text: string; readonly truncated: boolean }
  | {
      readonly ok: true;
      readonly capability: "http";
      readonly status: number;
      readonly body: string;
      readonly truncated: boolean;
    }
  | { readonly ok: true; readonly capability: "storage"; readonly operation: "get"; readonly value: string | null }
  | { readonly ok: true; readonly capability: "storage"; readonly operation: "set" | "delete"; readonly completed: true }
  | { readonly ok: true; readonly capability: "notification"; readonly delivered: true }
  | { readonly ok: true; readonly capability: "open"; readonly opened: true }
  | { readonly ok: true; readonly capability: "copy"; readonly copied: true }
  | CapabilityFailure;

/** Narrows a context call to success/failure responses for the requested capability. */
export type CapabilityResponseFor<Request extends CapabilityRequest> =
  | Extract<CapabilityResponse, { readonly ok: true; readonly capability: Request["capability"] }>
  | (Omit<CapabilityFailure, "capability"> & { readonly capability: Request["capability"] });
