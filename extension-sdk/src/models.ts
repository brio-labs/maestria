import type {
  CapabilityRequest,
  CapabilityResponseFor,
} from "./capabilities.js";

/** Host-enforced upper bounds for data crossing the extension boundary. */
export const VIEW_LIMITS = {
  titleCharacters: 120,
  textCharacters: 4_096,
  listItems: 100,
  detailBlocks: 32,
  formFields: 32,
  formChoices: 100,
  actions: 32,
  formValues: 32,
} as const;

export type FormValue = string | number | boolean | null;
export type FormValues = Readonly<Record<string, FormValue>>;

export interface ExtensionAction {
  readonly id: string;
  readonly label: string;
  readonly role?: "primary" | "secondary" | "destructive" | "submit" | "cancel";
}

export interface ListItem {
  /** Stable item ID, not a filesystem path or a callback. */
  readonly id: string;
  readonly title: string;
  readonly subtitle?: string;
  readonly actions?: readonly ExtensionAction[];
}

export interface ListView {
  readonly kind: "list";
  readonly title: string;
  readonly items: readonly ListItem[];
  readonly emptyMessage?: string;
  readonly actions?: readonly ExtensionAction[];
}

export type DetailBlock =
  | { readonly kind: "text"; readonly text: string }
  | {
      readonly kind: "properties";
      readonly rows: readonly {
        readonly label: string;
        readonly value: string;
      }[];
    };

export interface DetailView {
  readonly kind: "detail";
  readonly title: string;
  readonly blocks: readonly DetailBlock[];
  readonly actions?: readonly ExtensionAction[];
}

export type FormField =
  | {
      readonly kind: "text";
      readonly id: string;
      readonly label: string;
      readonly required?: boolean;
      readonly placeholder?: string;
      readonly initialValue?: string;
      readonly maxLength?: number;
    }
  | {
      readonly kind: "number";
      readonly id: string;
      readonly label: string;
      readonly required?: boolean;
      readonly min?: number;
      readonly max?: number;
      readonly step?: number;
      readonly initialValue?: number;
    }
  | {
      readonly kind: "select";
      readonly id: string;
      readonly label: string;
      readonly required?: boolean;
      readonly choices: readonly { readonly id: string; readonly label: string }[];
      readonly initialValue?: string;
    }
  | {
      readonly kind: "checkbox";
      readonly id: string;
      readonly label: string;
      readonly initialValue?: boolean;
    }
  | {
      /** The host presents a user-controlled picker and returns an opaque selection ID. */
      readonly kind: "file";
      readonly id: string;
      readonly label: string;
      readonly required?: boolean;
    };

export interface FormView {
  readonly kind: "form";
  readonly title: string;
  readonly fields: readonly FormField[];
  readonly actions: readonly ExtensionAction[];
}

export interface LoadingView {
  readonly kind: "loading";
  readonly message?: string;
}

export interface ErrorView {
  readonly kind: "error";
  readonly title?: string;
  readonly message: string;
  readonly actions?: readonly ExtensionAction[];
}

export interface ActionsView {
  readonly kind: "actions";
  readonly title: string;
  readonly actions: readonly ExtensionAction[];
}

/** Declarative data only; this API exposes no renderer, DOM node, or component. */
export type ExtensionView =
  | ListView
  | DetailView
  | FormView
  | LoadingView
  | ErrorView
  | ActionsView;

export type CommandInvocation =
  | { readonly kind: "command"; readonly input: FormValues }
  | {
      readonly kind: "action";
      readonly actionId: string;
      readonly itemId?: string;
      readonly values: FormValues;
    };

export interface ExtensionCommandContext {
  readonly extensionId: string;
  readonly commandId: string;
  readonly invocation: CommandInvocation;
  /** Every request is mediated by the host; extensions receive no ambient I/O. */
  readonly requestCapability: <Request extends CapabilityRequest>(
    request: Request,
  ) => Promise<CapabilityResponseFor<Request>>;
}

export type ExtensionCommandHandler = (
  context: ExtensionCommandContext,
) => ExtensionView | Promise<ExtensionView>;

/** Default export shape required from each manifest-declared entrypoint module. */
export interface ExtensionEntrypoint {
  readonly commands: Readonly<Record<string, ExtensionCommandHandler>>;
}
