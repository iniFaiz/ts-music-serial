import { describe, expect, it, vi } from 'vitest';

const { capturedRoutes } = vi.hoisted(() => ({
  capturedRoutes: [],
}));

vi.mock('vue-router', () => ({
  createWebHistory: () => ({ type: 'mock-history' }),
  createRouter: ({ routes }) => {
    capturedRoutes.splice(0, capturedRoutes.length, ...routes);
    return { routes };
  },
}));

await import('./router');

describe('router chunk loading', () => {
  it('declares every rendered route as a dynamic component loader', () => {
    const renderedRoutes = capturedRoutes.filter((route) => route.component);

    expect(renderedRoutes).toHaveLength(12);
    for (const route of renderedRoutes) {
      expect(route.component).toBeTypeOf('function');
      expect(route.component.constructor.name).toBe('Function');
    }
  });
});
