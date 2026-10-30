import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ErrorBoundary from "./ErrorBoundary";

/**
 * The boundary is a safety net, and the failure mode of an untested safety net
 * is that nobody finds out it does not hold until the day it is needed.
 *
 * React logs caught errors to the console itself, which would make the test
 * output look like a failure. Silenced per test rather than globally, so a
 * genuine unexpected console error still shows.
 */
describe("ErrorBoundary", () => {
  beforeEach(() => {
    vi.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  function Boom({ throws }: { throws: boolean }): React.ReactElement {
    if (throws) throw new Error("kaboom");
    return <div>the photo grid</div>;
  }

  it("renders its children untouched when nothing throws", () => {
    render(
      <ErrorBoundary>
        <Boom throws={false} />
      </ErrorBoundary>,
    );
    expect(screen.getByText("the photo grid")).toBeTruthy();
  });

  it("shows a way out instead of a blank window when a child throws", () => {
    render(
      <ErrorBoundary label="Develop">
        <Boom throws />
      </ErrorBoundary>,
    );

    // Named, so the user knows which part died rather than fearing all of it.
    expect(screen.getByText(/Develop stopped responding/)).toBeTruthy();
    expect(screen.getByText(/kaboom/)).toBeTruthy();
    // The three exits: retry, copy for a bug report, reload.
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy details" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reload" })).toBeTruthy();
  });

  it("reassures the user their library is intact", () => {
    render(
      <ErrorBoundary>
        <Boom throws />
      </ErrorBoundary>,
    );
    // The thing a photographer actually wants to know, and the thing this app
    // can honestly promise: it never writes to the originals.
    expect(screen.getByText(/ratings, tags and edits are safe/)).toBeTruthy();
  });

  it("carries the fallback class, so the direct-canvas rules treat it as Develop", () => {
    // The direct canvas hides the Library behind Develop. The fallback stands
    // in for Develop, so it has to be recognisable as Develop, not hidden with
    // what Develop covers.
    const { container } = render(
      <ErrorBoundary label="Develop" fallbackClassName="develop-shell">
        <Boom throws />
      </ErrorBoundary>,
    );
    expect(container.querySelector(".develop-shell")).toBeTruthy();
  });

  it("offers a way back to the library from an overlay that keeps failing", () => {
    const leave = vi.fn();
    render(
      <ErrorBoundary label="Develop" onLeave={leave}>
        <Boom throws />
      </ErrorBoundary>,
    );
    screen.getByText("Back to the library").click();
    expect(leave).toHaveBeenCalledOnce();
  });
});
