export type Zone = "domestic" | "international";
export type PackageType = "standard" | "fragile";

export interface Order {
  id: string;
  weightKg: number;
  zone: Zone;
  packageType: PackageType;
}
