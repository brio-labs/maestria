import type {
  ExtensionEntrypoint,
  ExtensionView,
  FormValues,
} from "@sillage/extension-sdk";

const CASE_STYLES = [
  { id: "uppercase", label: "UPPERCASE" },
  { id: "lowercase", label: "lowercase" },
  { id: "title", label: "Title Case" },
  { id: "camel", label: "camelCase" },
  { id: "snake", label: "snake_case" },
  { id: "kebab", label: "kebab-case" },
] as const;

type CaseStyle = (typeof CASE_STYLES)[number]["id"];

function stringValue(values: FormValues, id: string): string | undefined {
  const value = values[id];
  return typeof value === "string" ? value : undefined;
}

function capitalize(word: string): string {
  const [first, ...rest] = [...word.toLowerCase()];
  return `${first?.toUpperCase() ?? ""}${rest.join("")}`;
}

function convertText(text: string, style: CaseStyle): string {
  if (style === "uppercase") return text.toUpperCase();
  if (style === "lowercase") return text.toLowerCase();
  if (style === "title") {
    return text.toLowerCase().replace(/[\p{L}\p{N}\p{M}]+/gu, capitalize);
  }

  const separated = text
    .replace(/(\p{Lu})(\p{Lu}\p{Ll})/gu, "$1 $2")
    .replace(/([\p{Ll}\p{N}])(\p{Lu})/gu, "$1 $2");
  const words =
    separated.match(/[\p{L}\p{N}\p{M}]+/gu)?.map((word) => word.toLowerCase()) ?? [];
  switch (style) {
    case "camel":
      return words.map((word, index) => index === 0 ? word : capitalize(word)).join("");
    case "snake":
      return words.join("_");
    case "kebab":
      return words.join("-");
  }
}

function isCaseStyle(value: string): value is CaseStyle {
  return CASE_STYLES.some((style) => style.id === value);
}

function errorView(title: string, message: string): ExtensionView {
  return { kind: "error", title, message };
}

const entrypoint: ExtensionEntrypoint = {
  commands: {
    "text-tools": async ({ invocation, requestCapability }): Promise<ExtensionView> => {
      if (invocation.kind === "command") {
        return {
          kind: "form",
          title: "Text Tools",
          fields: [
            {
              kind: "text",
              id: "text",
              label: "Text to convert",
              required: true,
              placeholder: "Type or paste text",
              maxLength: 4096,
            },
            {
              kind: "select",
              id: "style",
              label: "Case style",
              required: true,
              choices: CASE_STYLES,
              initialValue: "uppercase",
            },
          ],
          actions: [{ id: "convert", label: "Convert", role: "submit" }],
        };
      }

      if (invocation.actionId === "convert") {
        const text = stringValue(invocation.values, "text");
        const style = stringValue(invocation.values, "style");
        if (
          text === undefined ||
          text.length === 0 ||
          style === undefined ||
          !isCaseStyle(style)
        ) {
          return errorView("Text required", "Enter text and select a supported case style.");
        }
        const converted = convertText(text, style);
        if (converted.length === 0) {
          return errorView(
            "Nothing to convert",
            "Enter at least one letter or number for this case style.",
          );
        }
        if (converted.length > 4096) {
          return errorView("Text is too long", "Shorten the input before converting it.");
        }
        return {
          kind: "form",
          title: "Review converted text",
          fields: [
            {
              kind: "text",
              id: "result",
              label: "Converted text",
              required: true,
              initialValue: converted,
              maxLength: 4096,
            },
          ],
          actions: [{ id: "copy-result", label: "Copy to clipboard", role: "primary" }],
        };
      }

      if (invocation.actionId === "copy-result") {
        const text = stringValue(invocation.values, "result");
        if (text === undefined || text.length === 0) {
          return errorView("Text required", "Enter text before copying it.");
        }
        const result = await requestCapability({ capability: "copy", text });
        if (!result.ok) {
          return errorView("Copy unavailable", result.error.message);
        }
        return {
          kind: "detail",
          title: "Text copied",
          blocks: [{ kind: "text", text: "The reviewed text was copied to the clipboard." }],
        };
      }

      return errorView("Unknown action", "Return to Text Tools and try the action again.");
    },
  },
};

export default entrypoint;
