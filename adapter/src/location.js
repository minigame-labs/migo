// location stub. The mini-game runtime has no real URL; engines that read
// location.href usually only check it's a string ending in .js.

const location = {
  href: "game.js",
  protocol: "https:",
  host: "game",
  hostname: "game",
  port: "",
  pathname: "/game.js",
  search: "",
  hash: "",
  origin: "https://game",
  reload() {},
  replace() {},
  assign() {},
  toString() { return this.href; },
};

export default location;
