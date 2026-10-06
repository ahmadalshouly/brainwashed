// The Worker's entry point. Cloudflare only allows handlers to be exported
// here; the logic lives in book.js.
import { fail, forgetStale, handle } from "./book.js";

export default {
  fetch: (request, env) =>
    handle(request, env).catch((e) => {
      console.error(e);
      return fail(500, "Something went wrong. Try again.");
    }),
  scheduled: (_event, env) => forgetStale(env),
};
