import { Button } from "./components/Button";
import { Card } from "./components/Card";
import { Item } from "./models/Item";

export { Button, Card };

export function createDefaultItem(): Item {
  return new Item("default", 1);
}

export const catalogSize = 3;
