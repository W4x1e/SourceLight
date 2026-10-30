import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ColorLabelDot, FlagBadge, RatingBadge } from "./PhotoBadges";

/** The grid and Compare now share these; pin what each variant draws. */
describe("PhotoBadges", () => {
  it("draws nothing for an unflagged, unrated, unlabelled photo", () => {
    const { container } = render(
      <>
        <FlagBadge flag={0} />
        <RatingBadge rating={0} />
        <ColorLabelDot label={0} />
      </>,
    );
    expect(container.innerHTML).toBe("");
  });

  it("marks a pick and a reject by shape as well as hue, with the word only when labelled", () => {
    const pick = render(<FlagBadge flag={1} />).container;
    const pickGlyph = pick.querySelector("svg.text-positive");
    expect(pickGlyph).toBeTruthy();
    expect(pick.textContent).toBe("");
    const reject = render(<FlagBadge flag={-1} labelled />).container;
    const rejectGlyph = reject.querySelector("svg.text-danger");
    expect(rejectGlyph).toBeTruthy();
    // Not one glyph in two colours: a filled flag, and a struck one.
    expect(pickGlyph!.innerHTML).not.toBe(rejectGlyph!.innerHTML);
    // The word is in ink, legible over any photo; the hue is on the glyph.
    expect(reject.textContent).toBe("reject");
    expect(reject.querySelector(".text-ink")).toBeTruthy();
  });

  it("draws one star per rating point and the label's own colour", () => {
    expect(render(<RatingBadge rating={3} />).container.textContent).toBe("★★★");
    const dot = render(<ColorLabelDot label={2} />).container.querySelector("span")!;
    expect(dot.title).toBeTruthy();
    expect(dot.style.backgroundColor).not.toBe("");
  });
});
