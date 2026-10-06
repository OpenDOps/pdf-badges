import "@testing-library/jest-dom/vitest";

import "./i18n";

class ImmediateObserver implements IntersectionObserver {
  readonly root: Element | Document | null = null;
  readonly rootMargin = "0px";
  readonly thresholds: ReadonlyArray<number>;
  constructor(
    private readonly callback: IntersectionObserverCallback,
    options?: IntersectionObserverInit,
  ) {
    const threshold = options?.threshold;
    this.thresholds = typeof threshold === "number" ? [threshold] : threshold && threshold.length > 0 ? threshold : [0];
  }
  observe(target: Element) {
    const ratio = 1;
    const entry = {
      isIntersecting: ratio >= (this.thresholds[0] ?? 0),
      intersectionRatio: ratio,
      target,
      boundingClientRect: target.getBoundingClientRect(),
      intersectionRect: target.getBoundingClientRect(),
      rootBounds: null,
      time: 0,
    } as IntersectionObserverEntry;
    this.callback([entry], this);
  }
  unobserve() {}
  disconnect() {}
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
}

beforeEach(() => {
  vi.stubGlobal("IntersectionObserver", ImmediateObserver);
});
