export function add(a: number, b: number): number {
  return a + b;
}

export function multiply(a: number, b: number): number {
  return a * b;
}

export function average(values: number[]): number {
  if (values.length === 0) {
    throw new Error("average of empty list");
  }
  return values.reduce((s, v) => s + v, 0) / values.length;
}
