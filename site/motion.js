// The page's motion: everything marked data-reveal fades up the first time it
// scrolls into view, and the nav gets its background once the page has moved.
//
// The inline script in index.html adds the `js` class that hides them to begin
// with, so without this file, or with reduced motion asked for, nothing is
// ever hidden.

const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

function reveal() {
  const items = document.querySelectorAll("[data-reveal]");
  if (reduced || !("IntersectionObserver" in window)) {
    items.forEach((el) => el.classList.add("in"));
    return;
  }
  // Siblings that arrive together come in one after another.
  items.forEach((el) => {
    const group = [...el.parentElement.children].filter((c) => c.hasAttribute("data-reveal"));
    el.style.setProperty("--delay", `${group.indexOf(el) * 90}ms`);
  });
  // The terminal's lines print one at a time once it is in view.
  document.querySelectorAll(".terminal .line").forEach((line, i) => line.style.setProperty("--i", i));
  const seen = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        const el = e.target;
        el.classList.add("in");
        seen.unobserve(el);
        // Once it is in, it drops the reveal's slow, staggered transition and
        // hovers with its own.
        el.addEventListener("transitionend", function done(t) {
          if (t.target !== el || t.propertyName !== "opacity") return;
          el.removeAttribute("data-reveal");
          el.removeEventListener("transitionend", done);
        });
      }
    },
    { rootMargin: "0px 0px -8% 0px", threshold: 0.12 },
  );
  items.forEach((el) => seen.observe(el));
}

function nav() {
  const bar = document.querySelector(".nav");
  const update = () => bar.classList.toggle("scrolled", scrollY > 8);
  addEventListener("scroll", update, { passive: true });
  update();
}

reveal();
nav();
