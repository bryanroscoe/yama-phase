// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";

afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

test("a host using text icons never requests the art font", async () => {
  vi.stubEnv("VITE_TEXT_ICONS", "1");
  vi.resetModules();
  const load = vi.fn().mockResolvedValue([]);
  const original = Object.getOwnPropertyDescriptor(document, "fonts");
  Object.defineProperty(document, "fonts", { configurable: true, value: { load } });
  try {
    const { ManaFontIcon } = await import("./ManaFontIcon");
    render(<ManaFontIcon iconClass="ms-ability-flying" fallbackText="Flying" />);
    expect(screen.getByText("Flying")).toBeVisible();
    expect(document.querySelector("i.ms")).toBeNull();
    expect(load).not.toHaveBeenCalled();
  } finally {
    if (original) Object.defineProperty(document, "fonts", original);
    else Reflect.deleteProperty(document, "fonts");
  }
});
