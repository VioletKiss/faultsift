import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import App from "./App";

describe("desktop shell", () => {
  it("renders the FaultSift application identity", () => {
    render(<App />);

    expect(screen.getByRole("heading", { name: "FaultSift" })).toBeVisible();
  });
});
