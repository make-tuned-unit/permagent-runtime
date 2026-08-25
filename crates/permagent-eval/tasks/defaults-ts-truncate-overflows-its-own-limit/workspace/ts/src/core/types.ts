export interface User {
  id: string;
  name: string;
  email: string;
  isAdmin: boolean;
}

export interface Todo {
  id: string;
  title: string;
  done: boolean;
  createdAt: number;
}

export interface CartItem {
  sku: string;
  name: string;
  priceCents: number;
  quantity: number;
}
