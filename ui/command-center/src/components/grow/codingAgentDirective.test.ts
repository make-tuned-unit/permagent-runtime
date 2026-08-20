import { describe, expect, it } from 'vitest';
import {
  GROW_ACTION_DONE_PREFIX,
  codingAgentDirective,
} from './codingAgentDirective';

const base = {
  projectName: 'GrocerySaver',
  projectRoot: '/Users/j/dev/grocerysaver',
};

describe('codingAgentDirective', () => {
  it('wraps a drafted SEO post as a coding-agent instruction, not bare copy', () => {
    const text = codingAgentDirective({
      ...base,
      action: {
        title: 'Publish a comparison post for grocery delivery',
        recommendation: 'Add a post that can rank for the query',
        evidence: '12 of 40 pageviews land on /blog',
        steps: ['Find the content folder', 'Add the post', 'Link it from the index'],
        artifactKind: 'post',
        artifact: 'Grocery delivery is expensive. Here is how to spend less.',
        category: 'seo',
        identity: {
          id: 'act-seo-1',
          targetMetric: 'pageviews',
          targetDir: 'up',
        },
      },
    });

    expect(text).toContain('coding agent');
    expect(text).toContain('GrocerySaver');
    expect(text).toContain('/Users/j/dev/grocerysaver');
    expect(text).toContain('Publish a comparison post');
    expect(text).toContain('Category: seo');
    expect(text).toContain('12 of 40 pageviews');
    expect(text).toContain('Find the content folder');
    expect(text).toContain('copy to publish');
    expect(text).toContain('Grocery delivery is expensive');
    expect(text).toContain('pageviews going up');
    expect(text).toContain(`${GROW_ACTION_DONE_PREFIX} act-seo-1`);
    // A bare post would be this line alone. The wrapper must outrank it.
    expect(text.trim().startsWith('Grocery delivery is expensive')).toBe(false);
  });

  it('keeps a prompt artifact inside a self-contained brief', () => {
    const text = codingAgentDirective({
      ...base,
      action: {
        title: 'Add FAQPage schema to /deals',
        recommendation: 'Put JSON-LD on the deals route',
        artifactKind: 'prompt',
        artifact: 'Add FAQPage JSON-LD to src/pages/deals.tsx',
        category: 'aeo',
      },
    });
    expect(text).toContain('Add FAQPage JSON-LD to src/pages/deals.tsx');
    expect(text).toContain('Follow the instruction');
    expect(text).not.toContain(`${GROW_ACTION_DONE_PREFIX} `);
  });

  it('still produces a prompt when there is no artifact', () => {
    const text = codingAgentDirective({
      projectName: 'GrocerySaver',
      action: {
        title: 'Decide whether to kill the deals funnel',
        recommendation: 'This is a human call, but record the decision in the repo if you make it.',
        artifactKind: 'none',
        artifact: null,
      },
    });
    expect(text).toContain('Decide whether to kill the deals funnel');
    expect(text).toContain('There is no separate artifact');
  });
});
