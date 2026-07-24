const views = Array.from(document.querySelectorAll("[data-view]"));
const links = Array.from(document.querySelectorAll("[data-view-link]"));

function showView(name) {
  const selected = views.some((view) => view.dataset.view === name) ? name : "overview";
  for (const view of views) {
    view.hidden = view.dataset.view !== selected;
  }
  for (const link of links) {
    if (link.dataset.viewLink === selected) {
      link.setAttribute("aria-current", "page");
    } else {
      link.removeAttribute("aria-current");
    }
  }
}

for (const link of links) {
  link.addEventListener("click", () => {
    showView(link.dataset.viewLink);
  });
}

document.querySelector("#show-evidence").addEventListener("click", (event) => {
  document.querySelector("#evidence-summary").hidden = false;
  event.currentTarget.textContent = "Evidence summary shown";
  event.currentTarget.disabled = true;
});

showView(window.location.hash.slice(1) || "overview");
