import type { ManifestPermission } from "./capabilities.js";

export const SDK_API_VERSION = 1 as const;

export const MANIFEST_LIMITS = {
  extensionIdCharacters: 128,
  extensionNameCharacters: 80,
  versionCharacters: 64,
  entrypoints: 64,
  commands: 128,
  permissions: 7,
  entrypointIdCharacters: 48,
  commandIdCharacters: 48,
  entrypointPathCharacters: 240,
  commandTitleCharacters: 80,
  commandDescriptionCharacters: 240,
  permissionOrigins: 32,
} as const;

export interface ManifestEntrypoint {
  readonly id: string;
  /** Package-relative JavaScript path; lexical checks do not replace host symlink checks. */
  readonly file: string;
}

export interface ManifestCommand {
  readonly id: string;
  readonly title: string;
  readonly entrypointId: string;
  readonly description?: string;
}

export interface ExtensionManifest {
  readonly apiVersion: typeof SDK_API_VERSION;
  readonly id: string;
  readonly name: string;
  readonly version: string;
  readonly entrypoints: readonly ManifestEntrypoint[];
  readonly commands: readonly ManifestCommand[];
  readonly permissions: readonly ManifestPermission[];
}

export type ManifestDiagnosticCode =
  | "INVALID_MANIFEST"
  | "UNKNOWN_FIELD"
  | "INVALID_API_VERSION"
  | "UNSUPPORTED_API_VERSION"
  | "INVALID_EXTENSION_ID"
  | "INVALID_EXTENSION_NAME"
  | "INVALID_EXTENSION_VERSION"
  | "INVALID_COLLECTION"
  | "TOO_MANY_ITEMS"
  | "INVALID_ENTRYPOINT_ID"
  | "INVALID_ENTRYPOINT_PATH"
  | "DUPLICATE_ID"
  | "DUPLICATE_ENTRYPOINT_FILE"
  | "INVALID_COMMAND_ID"
  | "INVALID_COMMAND_TITLE"
  | "INVALID_COMMAND_DESCRIPTION"
  | "INVALID_ENTRYPOINT_REFERENCE"
  | "INVALID_PERMISSION"
  | "DUPLICATE_PERMISSION"
  | "INVALID_PERMISSION_ORIGIN"
  | "DUPLICATE_PERMISSION_ORIGIN"
  | "INVALID_PERMISSION_VALUE";

export interface ManifestDiagnostic {
  readonly code: ManifestDiagnosticCode;
  /** JSON-style path to the rejected manifest value. */
  readonly path: string;
  readonly message: string;
}

export type ManifestValidationResult =
  | { readonly valid: true; readonly manifest: ExtensionManifest; readonly diagnostics: readonly [] }
  | { readonly valid: false; readonly diagnostics: readonly ManifestDiagnostic[] };

type JsonObject = Record<string, unknown>;
type DiagnosticList = ManifestDiagnostic[];

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function diagnostic(
  diagnostics: DiagnosticList,
  code: ManifestDiagnosticCode,
  path: string,
  message: string,
): void {
  diagnostics.push({ code, path, message });
}

function objectAt(
  value: unknown,
  path: string,
  allowedKeys: readonly string[],
  diagnostics: DiagnosticList,
): JsonObject | undefined {
  if (!isObject(value)) {
    diagnostic(diagnostics, "INVALID_MANIFEST", path, "Expected a JSON object.");
    return undefined;
  }

  for (const key of Object.keys(value)) {
    if (!allowedKeys.includes(key)) {
      diagnostic(
        diagnostics,
        "UNKNOWN_FIELD",
        `${path}.${key}`,
        `Unknown field ${JSON.stringify(key)}; remove it or use a supported API version.`,
      );
    }
  }
  return value;
}

function textAt(
  object: JsonObject,
  key: string,
  path: string,
  maximum: number,
  code: ManifestDiagnosticCode,
  label: string,
  diagnostics: DiagnosticList,
): string | undefined {
  const value = object[key];
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > maximum ||
    value.trim() !== value ||
    /[\u0000-\u001f\u007f]/u.test(value)
  ) {
    diagnostic(
      diagnostics,
      code,
      `${path}.${key}`,
      `${label} must be non-empty, trimmed text of at most ${maximum} characters with no control characters.`,
    );
    return undefined;
  }
  return value;
}

function identifierAt(
  object: JsonObject,
  key: string,
  path: string,
  maximum: number,
  pattern: RegExp,
  code: ManifestDiagnosticCode,
  label: string,
  diagnostics: DiagnosticList,
): string | undefined {
  const value = object[key];
  if (typeof value !== "string" || value.length > maximum || !pattern.test(value)) {
    diagnostic(
      diagnostics,
      code,
      `${path}.${key}`,
      `${label} must be a lowercase path-independent identifier of at most ${maximum} characters; use letters, digits, and the documented separators only.`,
    );
    return undefined;
  }
  return value;
}

function arrayAt(
  object: JsonObject,
  key: string,
  path: string,
  maximum: number,
  minimum: number,
  diagnostics: DiagnosticList,
): readonly unknown[] | undefined {
  const value = object[key];
  if (!Array.isArray(value)) {
    diagnostic(diagnostics, "INVALID_COLLECTION", `${path}.${key}`, "Expected an array.");
    return undefined;
  }
  if (value.length < minimum) {
    diagnostic(
      diagnostics,
      "INVALID_COLLECTION",
      `${path}.${key}`,
      `Provide at least ${minimum} ${key === "entrypoints" ? "entrypoint" : key === "commands" ? "command" : "item"}${minimum === 1 ? "" : "s"}.`,
    );
  }
  if (value.length > maximum) {
    diagnostic(
      diagnostics,
      "TOO_MANY_ITEMS",
      `${path}.${key}`,
      `At most ${maximum} ${key} are allowed.`,
    );
  }
  return value;
}

function optionalTextAt(
  object: JsonObject,
  key: string,
  path: string,
  maximum: number,
  code: ManifestDiagnosticCode,
  label: string,
  diagnostics: DiagnosticList,
): string | undefined {
  if (!Object.prototype.hasOwnProperty.call(object, key)) return undefined;
  return textAt(object, key, path, maximum, code, label, diagnostics);
}

function optionalIntegerAt(
  object: JsonObject,
  key: string,
  path: string,
  minimum: number,
  maximum: number,
  diagnostics: DiagnosticList,
): number | undefined {
  if (!Object.prototype.hasOwnProperty.call(object, key)) return undefined;
  const value = object[key];
  if (typeof value !== "number" || !Number.isInteger(value) || value < minimum || value > maximum) {
    diagnostic(
      diagnostics,
      "INVALID_PERMISSION_VALUE",
      `${path}.${key}`,
      `Expected an integer from ${minimum} through ${maximum}.`,
    );
    return undefined;
  }
  return value;
}

function isSemanticVersion(value: string): boolean {
  const match = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/u.exec(value);
  if (match === null) return false;
  const prerelease = match[4];
  return prerelease === undefined || !prerelease.split(".").some(
    (part) => /^[0-9]+$/u.test(part) && part.length > 1 && part.startsWith("0"),
  );
}

function isPackageRelativeJavaScriptPath(value: string): boolean {
  if (
    value.length === 0 ||
    value.length > MANIFEST_LIMITS.entrypointPathCharacters ||
    value.startsWith("/") ||
    value.includes("\\") ||
    value.includes(":") ||
    value.includes("%") ||
    !value.endsWith(".js")
  ) {
    return false;
  }
  const segments = value.split("/");
  return segments.length <= 16 && segments.every(
    (segment) =>
      segment !== "." &&
      segment !== ".." &&
      /^[A-Za-z0-9_-][A-Za-z0-9._-]*$/u.test(segment),
  );
}

function validHttpsOrigin(value: string): boolean {
  if (value.length > 255) return false;
  const match = /^https:\/\/([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)*)(?::([1-9][0-9]{0,4}))?$/u.exec(value);
  if (match === null || match[1].length > 253) return false;
  return match[2] === undefined || Number(match[2]) <= 65_535;
}

function enumArrayAt<T extends string>(
  object: JsonObject,
  key: string,
  path: string,
  maximum: number,
  allowed: readonly T[],
  diagnostics: DiagnosticList,
): readonly T[] | undefined {
  const value = object[key];
  if (!Array.isArray(value) || value.length === 0 || value.length > maximum) {
    diagnostic(
      diagnostics,
      value && Array.isArray(value) && value.length > maximum ? "TOO_MANY_ITEMS" : "INVALID_PERMISSION_VALUE",
      `${path}.${key}`,
      `Provide between 1 and ${maximum} supported ${key} value${maximum === 1 ? "" : "s"}.`,
    );
    return undefined;
  }
  const selected: T[] = [];
  const seen = new Set<string>();
  for (let index = 0; index < value.length; index += 1) {
    const item = value[index];
    if (typeof item !== "string" || !allowed.includes(item as T)) {
      diagnostic(
        diagnostics,
        "INVALID_PERMISSION_VALUE",
        `${path}.${key}[${index}]`,
        `Use one of: ${allowed.join(", ")}.`,
      );
      continue;
    }
    if (seen.has(item)) {
      diagnostic(
        diagnostics,
        "INVALID_PERMISSION_VALUE",
        `${path}.${key}[${index}]`,
        `Duplicate ${key} value ${JSON.stringify(item)}; list each value once.`,
      );
      continue;
    }
    seen.add(item);
    selected.push(item as T);
  }
  return selected;
}

function parseEntrypoint(value: unknown, index: number, diagnostics: DiagnosticList): ManifestEntrypoint | undefined {
  const path = `$.entrypoints[${index}]`;
  const before = diagnostics.length;
  const object = objectAt(value, path, ["id", "file"], diagnostics);
  if (object === undefined) return undefined;
  const id = identifierAt(
    object,
    "id",
    path,
    MANIFEST_LIMITS.entrypointIdCharacters,
    /^[a-z][a-z0-9_-]*$/u,
    "INVALID_ENTRYPOINT_ID",
    "Entrypoint ID",
    diagnostics,
  );
  const file = textAt(
    object,
    "file",
    path,
    MANIFEST_LIMITS.entrypointPathCharacters,
    "INVALID_ENTRYPOINT_PATH",
    "Entrypoint file path",
    diagnostics,
  );
  if (file !== undefined && !isPackageRelativeJavaScriptPath(file)) {
    diagnostic(
      diagnostics,
      "INVALID_ENTRYPOINT_PATH",
      `${path}.file`,
      "Use a package-relative .js path with safe slash-separated segments; absolute paths, `.`/`..`, backslashes, encoded separators, and non-JavaScript files are rejected. This lexical check does not inspect symlinks: the host must reject symlink escapes before execution.",
    );
  }
  if (diagnostics.length !== before || id === undefined || file === undefined) return undefined;
  return { id, file };
}

function parseCommand(value: unknown, index: number, diagnostics: DiagnosticList): ManifestCommand | undefined {
  const path = `$.commands[${index}]`;
  const before = diagnostics.length;
  const object = objectAt(value, path, ["id", "title", "entrypointId", "description"], diagnostics);
  if (object === undefined) return undefined;
  const id = identifierAt(
    object,
    "id",
    path,
    MANIFEST_LIMITS.commandIdCharacters,
    /^[a-z][a-z0-9_-]*$/u,
    "INVALID_COMMAND_ID",
    "Command ID",
    diagnostics,
  );
  const title = textAt(
    object,
    "title",
    path,
    MANIFEST_LIMITS.commandTitleCharacters,
    "INVALID_COMMAND_TITLE",
    "Command title",
    diagnostics,
  );
  const entrypointId = identifierAt(
    object,
    "entrypointId",
    path,
    MANIFEST_LIMITS.entrypointIdCharacters,
    /^[a-z][a-z0-9_-]*$/u,
    "INVALID_ENTRYPOINT_REFERENCE",
    "Command entrypoint ID",
    diagnostics,
  );
  const description = optionalTextAt(
    object,
    "description",
    path,
    MANIFEST_LIMITS.commandDescriptionCharacters,
    "INVALID_COMMAND_DESCRIPTION",
    "Command description",
    diagnostics,
  );
  if (diagnostics.length !== before || id === undefined || title === undefined || entrypointId === undefined) {
    return undefined;
  }
  return description === undefined ? { id, title, entrypointId } : { id, title, entrypointId, description };
}

function parsePermission(value: unknown, index: number, diagnostics: DiagnosticList): ManifestPermission | undefined {
  const path = `$.permissions[${index}]`;
  const before = diagnostics.length;
  if (!isObject(value)) {
    diagnostic(diagnostics, "INVALID_PERMISSION", path, "Expected a permission descriptor object.");
    return undefined;
  }
  const type = value.type;
  if (typeof type !== "string") {
    diagnostic(
      diagnostics,
      "INVALID_PERMISSION",
      `${path}.type`,
      "Permission needs a supported `type`: fileSearch, userFileRead, http, storage, notification, open, or copy.",
    );
    return undefined;
  }

  let permission: ManifestPermission | undefined;
  switch (type) {
    case "fileSearch": {
      const object = objectAt(value, path, ["type", "maxResults"], diagnostics);
      if (object === undefined) return undefined;
      const maxResults = optionalIntegerAt(object, "maxResults", path, 1, 100, diagnostics);
      permission = maxResults === undefined ? { type } : { type, maxResults };
      break;
    }
    case "userFileRead": {
      const object = objectAt(value, path, ["type"], diagnostics);
      if (object === undefined) return undefined;
      permission = { type };
      break;
    }
    case "http": {
      const object = objectAt(value, path, ["type", "origins"], diagnostics);
      if (object === undefined) return undefined;
      const originsValue = object.origins;
      if (!Array.isArray(originsValue) || originsValue.length === 0 || originsValue.length > MANIFEST_LIMITS.permissionOrigins) {
        diagnostic(
          diagnostics,
          originsValue && Array.isArray(originsValue) && originsValue.length > MANIFEST_LIMITS.permissionOrigins
            ? "TOO_MANY_ITEMS"
            : "INVALID_PERMISSION_VALUE",
          `${path}.origins`,
          `HTTP permission needs 1 to ${MANIFEST_LIMITS.permissionOrigins} exact HTTPS origins (no wildcard or path).`,
        );
        break;
      }
      const origins: string[] = [];
      const seen = new Set<string>();
      for (let originIndex = 0; originIndex < originsValue.length; originIndex += 1) {
        const origin = originsValue[originIndex];
        if (typeof origin !== "string" || !validHttpsOrigin(origin)) {
          diagnostic(
            diagnostics,
            "INVALID_PERMISSION_ORIGIN",
            `${path}.origins[${originIndex}]`,
            "Use a lowercase exact HTTPS origin such as `https://api.example.com`; wildcards, credentials, paths, query strings, and fragments are not allowed.",
          );
          continue;
        }
        if (seen.has(origin)) {
          diagnostic(
            diagnostics,
            "DUPLICATE_PERMISSION_ORIGIN",
            `${path}.origins[${originIndex}]`,
            `Origin ${JSON.stringify(origin)} is repeated; list each permitted origin once.`,
          );
          continue;
        }
        seen.add(origin);
        origins.push(origin);
      }
      permission = { type, origins };
      break;
    }
    case "storage": {
      const object = objectAt(value, path, ["type", "scope"], diagnostics);
      if (object === undefined) return undefined;
      if (object.scope !== "extension") {
        diagnostic(diagnostics, "INVALID_PERMISSION_VALUE", `${path}.scope`, "Storage scope must be exactly `extension`.");
      }
      permission = { type, scope: "extension" };
      break;
    }
    case "notification": {
      const object = objectAt(value, path, ["type"], diagnostics);
      if (object === undefined) return undefined;
      permission = { type };
      break;
    }
    case "open": {
      const object = objectAt(value, path, ["type", "targets"], diagnostics);
      if (object === undefined) return undefined;
      const targets = enumArrayAt(object, "targets", path, 2, ["url", "selectedFile"] as const, diagnostics);
      if (targets !== undefined) permission = { type, targets };
      break;
    }
    case "copy": {
      const object = objectAt(value, path, ["type", "formats"], diagnostics);
      if (object === undefined) return undefined;
      const formats = enumArrayAt(object, "formats", path, 1, ["text"] as const, diagnostics);
      if (formats !== undefined) permission = { type, formats };
      break;
    }
    default:
      diagnostic(
        diagnostics,
        "INVALID_PERMISSION",
        `${path}.type`,
        `Unsupported permission ${JSON.stringify(type)}. Supported types are fileSearch, userFileRead, http, storage, notification, open, and copy.`,
      );
      return undefined;
  }

  if (diagnostics.length !== before || permission === undefined) return undefined;
  return permission;
}

/**
 * Strictly validates an untrusted JSON manifest. It checks lexical entrypoint
 * paths only; installation must independently enforce bundle containment and
 * reject symlinks before executing any entrypoint.
 */
export function validateManifest(input: unknown): ManifestValidationResult {
  const diagnostics: DiagnosticList = [];
  const root = objectAt(
    input,
    "$",
    ["apiVersion", "id", "name", "version", "entrypoints", "commands", "permissions"],
    diagnostics,
  );
  if (root === undefined) return { valid: false, diagnostics };

  if (typeof root.apiVersion !== "number" || !Number.isInteger(root.apiVersion) || root.apiVersion < 1) {
    diagnostic(
      diagnostics,
      "INVALID_API_VERSION",
      "$.apiVersion",
      "`apiVersion` must be a positive integer. Set it to 1 for the current Sillage SDK.",
    );
  } else if (root.apiVersion !== SDK_API_VERSION) {
    diagnostic(
      diagnostics,
      "UNSUPPORTED_API_VERSION",
      "$.apiVersion",
      `Manifest requests SDK API ${root.apiVersion}; this SDK supports API ${SDK_API_VERSION}. Update the manifest or install a compatible host/SDK instead of falling back silently.`,
    );
  }

  const id = identifierAt(
    root,
    "id",
    "$",
    MANIFEST_LIMITS.extensionIdCharacters,
    /^[a-z][a-z0-9]*(?:[.-][a-z0-9]+)*$/u,
    "INVALID_EXTENSION_ID",
    "Extension ID",
    diagnostics,
  );
  const name = textAt(
    root,
    "name",
    "$",
    MANIFEST_LIMITS.extensionNameCharacters,
    "INVALID_EXTENSION_NAME",
    "Extension name",
    diagnostics,
  );
  const version = textAt(
    root,
    "version",
    "$",
    MANIFEST_LIMITS.versionCharacters,
    "INVALID_EXTENSION_VERSION",
    "Extension version",
    diagnostics,
  );
  if (version !== undefined && !isSemanticVersion(version)) {
    diagnostic(
      diagnostics,
      "INVALID_EXTENSION_VERSION",
      "$.version",
      "Use a semantic version such as `1.2.3` (optional prerelease/build identifiers are supported).",
    );
  }

  const entrypointValues = arrayAt(root, "entrypoints", "$", MANIFEST_LIMITS.entrypoints, 1, diagnostics);
  const commandValues = arrayAt(root, "commands", "$", MANIFEST_LIMITS.commands, 1, diagnostics);
  const permissionValues = arrayAt(root, "permissions", "$", MANIFEST_LIMITS.permissions, 0, diagnostics);
  const entrypoints: ManifestEntrypoint[] = [];
  const commands: ManifestCommand[] = [];
  const permissions: ManifestPermission[] = [];
  const entrypointIds = new Set<string>();
  const entrypointFiles = new Set<string>();
  const commandIds = new Set<string>();
  const permissionTypes = new Set<string>();

  if (entrypointValues !== undefined) {
    const count = Math.min(entrypointValues.length, MANIFEST_LIMITS.entrypoints);
    for (let index = 0; index < count; index += 1) {
      const entrypoint = parseEntrypoint(entrypointValues[index], index, diagnostics);
      if (entrypoint === undefined) continue;
      if (entrypointIds.has(entrypoint.id)) {
        diagnostic(
          diagnostics,
          "DUPLICATE_ID",
          `$.entrypoints[${index}].id`,
          `Entrypoint ID ${JSON.stringify(entrypoint.id)} is duplicated; every entrypoint in an extension needs a unique ID.`,
        );
      } else {
        entrypointIds.add(entrypoint.id);
      }
      if (entrypointFiles.has(entrypoint.file)) {
        diagnostic(
          diagnostics,
          "DUPLICATE_ENTRYPOINT_FILE",
          `$.entrypoints[${index}].file`,
          `Entrypoint file ${JSON.stringify(entrypoint.file)} is already declared; point each entrypoint at its own module file.`,
        );
      } else {
        entrypointFiles.add(entrypoint.file);
      }
      entrypoints.push(entrypoint);
    }
  }

  if (commandValues !== undefined) {
    const count = Math.min(commandValues.length, MANIFEST_LIMITS.commands);
    for (let index = 0; index < count; index += 1) {
      const command = parseCommand(commandValues[index], index, diagnostics);
      if (command === undefined) continue;
      if (commandIds.has(command.id)) {
        diagnostic(
          diagnostics,
          "DUPLICATE_ID",
          `$.commands[${index}].id`,
          `Command ID ${JSON.stringify(command.id)} is duplicated; every command in an extension needs a unique ID.`,
        );
      } else {
        commandIds.add(command.id);
      }
      if (!entrypointIds.has(command.entrypointId)) {
        diagnostic(
          diagnostics,
          "INVALID_ENTRYPOINT_REFERENCE",
          `$.commands[${index}].entrypointId`,
          `Entrypoint ${JSON.stringify(command.entrypointId)} is not declared; add it to \`entrypoints\` or correct this reference.`,
        );
      }
      commands.push(command);
    }
  }

  if (permissionValues !== undefined) {
    const count = Math.min(permissionValues.length, MANIFEST_LIMITS.permissions);
    for (let index = 0; index < count; index += 1) {
      const permission = parsePermission(permissionValues[index], index, diagnostics);
      if (permission === undefined) continue;
      if (permissionTypes.has(permission.type)) {
        diagnostic(
          diagnostics,
          "DUPLICATE_PERMISSION",
          `$.permissions[${index}].type`,
          `Permission ${JSON.stringify(permission.type)} is declared more than once; combine its scopes/origins into one descriptor.`,
        );
      } else {
        permissionTypes.add(permission.type);
      }
      permissions.push(permission);
    }
  }

  if (
    diagnostics.length !== 0 ||
    id === undefined ||
    name === undefined ||
    version === undefined ||
    root.apiVersion !== SDK_API_VERSION
  ) {
    return { valid: false, diagnostics };
  }

  return {
    valid: true,
    manifest: {
      apiVersion: SDK_API_VERSION,
      id,
      name,
      version,
      entrypoints,
      commands,
      permissions,
    },
    diagnostics: [],
  };
}
