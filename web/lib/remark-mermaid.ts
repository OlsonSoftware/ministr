import type { Root, Code } from 'mdast';
import type { Plugin } from 'unified';
import { visit } from 'unist-util-visit';

/**
 * Turn ```mermaid fenced code blocks into <Mermaid chart="..." /> JSX so the
 * client component can render them as actual diagrams. Shiki strips the
 * language-mermaid class before render, so DOM-level detection doesn't work —
 * we have to rewrite the mdast node here.
 */
export const remarkMermaid: Plugin<[], Root> = () => {
  return (tree) => {
    visit(tree, 'code', (node: Code, index, parent) => {
      if (node.lang !== 'mermaid' || !parent || index === undefined) return;
      const value = node.value ?? '';
      const jsxNode = {
        type: 'mdxJsxFlowElement' as const,
        name: 'Mermaid',
        attributes: [
          {
            type: 'mdxJsxAttribute' as const,
            name: 'chart',
            value,
          },
        ],
        children: [],
      };
      parent.children.splice(index, 1, jsxNode as never);
    });
  };
};
