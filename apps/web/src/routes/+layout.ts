/**
 * Root universal layout.
 *
 * The web app is a static SPA served by the Rust server (D4): every route renders
 * in the browser and reads its data from `/api/*`. Nothing is prerendered, so the
 * adapter's `index.html` fallback serves every path.
 */

export const ssr = false;
