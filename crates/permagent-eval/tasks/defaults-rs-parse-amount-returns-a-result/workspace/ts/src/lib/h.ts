export type VNode = {
  tag: string;
  props: Record<string, unknown>;
  children: (VNode | string)[];
};

export function h(
  tag: string,
  props: Record<string, unknown> = {},
  ...children: (VNode | string)[]
): VNode {
  return { tag, props, children };
}
