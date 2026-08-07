export class Item {
  name: string;
  price: number;

  constructor(name: string, price: number) {
    this.name = name;
    this.price = price;
  }

  total(quantity: number): number {
    return this.price * quantity;
  }
}

export function makeItem(name: string, price = 1) {
  return new Item(name, price);
}
