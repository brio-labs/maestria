import { Button } from "../src/components/Button";

export function renderButton(): string {
  const label = "Go";
  return Button({ label }).label;
}
