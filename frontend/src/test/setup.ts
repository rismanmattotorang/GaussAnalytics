// Test environment shims. jsdom lacks ResizeObserver, which Nivo's responsive
// chart wrappers instantiate on mount; without this they would throw. The no-op
// observer lets charts mount cleanly in tests (they render at zero size, so
// component tests assert on the data/table path rather than chart internals).
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

const g = globalThis as { ResizeObserver?: unknown };
g.ResizeObserver = g.ResizeObserver ?? ResizeObserverStub;
