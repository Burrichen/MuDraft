import "@testing-library/jest-dom/vitest";

// jsdom lacks modal <dialog> support; the WebViews MuDraft ships on (WebKit, WebView2) have it.
// Minimal stand-in so components using the native API can be tested.
if (typeof HTMLDialogElement.prototype.showModal !== "function") {
  HTMLDialogElement.prototype.showModal = function showModal(this: HTMLDialogElement) {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function close(this: HTMLDialogElement) {
    this.open = false;
    this.dispatchEvent(new Event("close"));
  };
}
