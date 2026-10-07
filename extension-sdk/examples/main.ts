import type { ExtensionEntrypoint, ExtensionView } from "@sillage/extension-sdk";

const entrypoint: ExtensionEntrypoint = {
  commands: {
    greetings: async ({ invocation, requestCapability }): Promise<ExtensionView> => {
      if (invocation.kind === "action" && invocation.actionId === "copy-greeting") {
        const result = await requestCapability({
          capability: "copy",
          text: "Hello from Sillage!",
        });
        if (!result.ok) {
          return { kind: "error", title: "Copy unavailable", message: result.error.message };
        }
        return {
          kind: "detail",
          title: "Greeting copied",
          blocks: [{ kind: "text", text: "Hello from Sillage!" }],
        };
      }

      const result = await requestCapability({
        capability: "fileSearch",
        query: "greeting",
        limit: 20,
      });
      if (!result.ok) {
        return { kind: "error", title: "Search unavailable", message: result.error.message };
      }
      return {
        kind: "list",
        title: "Greetings",
        emptyMessage: "No greeting matches were found.",
        items: result.results.map((item) => ({
          id: item.fileId,
          title: item.title,
          subtitle: item.snippet,
          actions: [{ id: "copy-greeting", label: "Copy a greeting", role: "primary" }],
        })),
      };
    },
  },
};

export default entrypoint;
