// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from "@testing-library/react";
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

test("text icons keep type badges, tapping and loyalty frames visible with empty caller fallbacks", async () => {
  vi.stubEnv("VITE_TEXT_ICONS", "1");
  vi.resetModules();
  const { ManaFontIcon } = await import("./ManaFontIcon");
  const { TypeIconBadge } = await import("../board/TypeIconBadge");
  const { LoyaltyBadge } = await import("../ui/LoyaltyBadge");

  render(
    <>
      <TypeIconBadge coreTypes={["Artifact", "Creature"]} />
      <ManaFontIcon iconClass="ms-tap" fallbackText="" />
      <LoyaltyBadge amount={3} kind="cost" />
      <LoyaltyBadge amount={-2} kind="cost" />
      <LoyaltyBadge amount={0} kind="cost" />
      <LoyaltyBadge amount={4} kind="total" />
    </>,
  );

  expect(screen.getByRole("img", { name: "Artifact" })).toHaveTextContent("Artifact");
  expect(screen.getByRole("img", { name: "Creature" })).toHaveTextContent("Creature");
  expect(screen.getByText("↷")).toBeVisible();
  expect(screen.getAllByText("△")).toHaveLength(3);
  expect(screen.getAllByText("▽")).toHaveLength(3);
  expect(screen.getAllByText("◇")).toHaveLength(6);
  for (const value of ["+3", "−2", "0", "4"]) {
    expect(screen.getByRole("img", { name: value })).toHaveTextContent(value);
  }
  expect(document.querySelector("i.ms")).toBeNull();
});

test("ordinary builds still load and render the icon font", async () => {
  vi.stubEnv("VITE_TEXT_ICONS", "0");
  vi.resetModules();
  const load = vi.fn().mockResolvedValue([]);
  const original = Object.getOwnPropertyDescriptor(document, "fonts");
  Object.defineProperty(document, "fonts", { configurable: true, value: { load } });
  try {
    const { ManaFontIcon } = await import("./ManaFontIcon");
    render(<ManaFontIcon iconClass="ms-tap" fallbackText="" />);
    await waitFor(() => expect(document.querySelector("i.ms-tap")).not.toBeNull());
    expect(load).toHaveBeenCalledExactlyOnceWith("1em Mana");
    expect(screen.queryByText("↷")).toBeNull();
  } finally {
    if (original) Object.defineProperty(document, "fonts", original);
    else Reflect.deleteProperty(document, "fonts");
  }
});
