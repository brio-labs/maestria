function pad(value: number): string {
  return String(value).padStart(2, "0");
}

export function formatPrice(cents: number): string {
  const value = pad(cents / 100);
  return `$${value}`;
}
