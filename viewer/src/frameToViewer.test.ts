import { describe, expect, it } from "vitest";
import { toViewerReferenceFrame } from "./frameToViewer.js";

describe("toViewerReferenceFrame", () => {
  it("maps central-body inertial", () => {
    expect(
      toViewerReferenceFrame({ center: { type: "central_body" }, orientation: "inertial" }),
    ).toEqual({ center: "centralBody", orientation: "inertial" });
  });

  it("maps central-body body-fixed -> bodyFixed", () => {
    expect(
      toViewerReferenceFrame({ center: { type: "central_body" }, orientation: "body_fixed" }),
    ).toEqual({ center: "centralBody", orientation: "bodyFixed" });
  });

  it("maps satellite inertial", () => {
    expect(
      toViewerReferenceFrame({
        center: { type: "satellite", id: "sat-1" },
        orientation: "inertial",
      }),
    ).toEqual({ center: { satelliteId: "sat-1" }, orientation: "inertial" });
  });

  it("maps satellite local_orbital -> localOrbital", () => {
    expect(
      toViewerReferenceFrame({
        center: { type: "satellite", id: "sat-1" },
        orientation: "local_orbital",
      }),
    ).toEqual({ center: { satelliteId: "sat-1" }, orientation: "localOrbital" });
  });

  it("falls back to central-body inertial for unsupported body centres", () => {
    expect(toViewerReferenceFrame({ center: { type: "moon" }, orientation: "inertial" })).toEqual({
      center: "centralBody",
      orientation: "inertial",
    });
  });
});
