import { describe, expect, it } from "vitest";
import {
  GIST_CAPACITY_ERROR_CODE,
  isGistCapacitySyncFailure,
  isGistCloudProvider,
  localizeCloudSyncMessage,
  needsGistCapacityRecovery,
} from "@/lib/cloudSync";

const NOT_ACCEPTED_MESSAGE =
  "New sync snapshot was not accepted by remote storage (candidate revision 45a349e7; latest was not changed).";
const REJECTED_FILE_MESSAGE =
  "Remote sync storage rejected file 'nyaterm-x.blob' (gist may be at file capacity).";

describe("cloud sync gist capacity helpers", () => {
  it("recognises gist-style providers", () => {
    expect(isGistCloudProvider("gitee_snippet")).toBe(true);
    expect(isGistCloudProvider("github_gist")).toBe(true);
    expect(isGistCloudProvider("webdav")).toBe(false);
    expect(isGistCloudProvider("s3")).toBe(false);
    expect(isGistCloudProvider(null)).toBe(false);
    expect(isGistCloudProvider(undefined)).toBe(false);
  });

  describe("cloud sync message localization", () => {
    it("translates known backend status messages", () => {
      expect(
        localizeCloudSyncMessage(
          "Cloud sync is up to date",
          (key) => `[${key}]`,
        ),
      ).toBe("[settings.syncMessageUpToDate]");
      expect(
        localizeCloudSyncMessage(
          "Cloud sync snapshot uploaded",
          (key) => `[${key}]`,
        ),
      ).toBe("[settings.syncMessageSnapshotUploaded]");
    });

    it("preserves unknown backend messages", () => {
      const message = "Remote storage returned an unexpected response.";
      expect(localizeCloudSyncMessage(message, (key) => `[${key}]`)).toBe(message);
    });
  });

  it("matches the legacy capacity failure messages", () => {
    expect(isGistCapacitySyncFailure(NOT_ACCEPTED_MESSAGE)).toBe(true);
    expect(isGistCapacitySyncFailure(REJECTED_FILE_MESSAGE)).toBe(true);
    expect(
      isGistCapacitySyncFailure(
        "Remote sync snapshot revision mismatch: latest points to r1 but snapshot contains r2.",
      ),
    ).toBe(false);
    expect(isGistCapacitySyncFailure("")).toBe(false);
    expect(isGistCapacitySyncFailure(null)).toBe(false);
    expect(isGistCapacitySyncFailure(undefined)).toBe(false);
  });

  it("prefers the status error code over any prose match", () => {
    const rewordedMessage = "The snippet could not take another object.";

    expect(
      needsGistCapacityRecovery("gitee_snippet", rewordedMessage, GIST_CAPACITY_ERROR_CODE),
    ).toBe(true);
    expect(needsGistCapacityRecovery("gitee_snippet", "", GIST_CAPACITY_ERROR_CODE)).toBe(true);
  });

  it("falls back to the message when the status carries no code", () => {
    expect(needsGistCapacityRecovery("gitee_snippet", NOT_ACCEPTED_MESSAGE, null)).toBe(true);
    expect(needsGistCapacityRecovery("gitee_snippet", REJECTED_FILE_MESSAGE, undefined)).toBe(true);
  });

  it.each([
    NOT_ACCEPTED_MESSAGE,
    REJECTED_FILE_MESSAGE,
  ])("does not offer GitHub Gist capacity recovery for %s", (message) => {
    expect(needsGistCapacityRecovery("github_gist", message)).toBe(false);
    expect(needsGistCapacityRecovery("github_gist", message, null)).toBe(false);
    expect(needsGistCapacityRecovery("github_gist", message, GIST_CAPACITY_ERROR_CODE)).toBe(false);
  });

  it("only offers capacity recovery for Gitee Snippet", () => {
    expect(needsGistCapacityRecovery("gitee_snippet", NOT_ACCEPTED_MESSAGE)).toBe(true);
    expect(needsGistCapacityRecovery("github_gist", "", GIST_CAPACITY_ERROR_CODE)).toBe(false);
    expect(needsGistCapacityRecovery("webdav", NOT_ACCEPTED_MESSAGE)).toBe(false);
    expect(needsGistCapacityRecovery("s3", REJECTED_FILE_MESSAGE)).toBe(false);
    expect(needsGistCapacityRecovery(null, NOT_ACCEPTED_MESSAGE)).toBe(false);
    expect(needsGistCapacityRecovery(undefined, REJECTED_FILE_MESSAGE)).toBe(false);
    expect(
      needsGistCapacityRecovery("webdav", NOT_ACCEPTED_MESSAGE, GIST_CAPACITY_ERROR_CODE),
    ).toBe(false);
    expect(
      needsGistCapacityRecovery("gitee_snippet", "Remote sync was updated by another device:"),
    ).toBe(false);
  });
});
