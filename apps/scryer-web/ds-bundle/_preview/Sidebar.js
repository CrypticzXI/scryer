"use strict";
var __dsPreview = (() => {
  var __create = Object.create;
  var __defProp = Object.defineProperty;
  var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
  var __getOwnPropNames = Object.getOwnPropertyNames;
  var __getProtoOf = Object.getPrototypeOf;
  var __hasOwnProp = Object.prototype.hasOwnProperty;
  var __esm = (fn, res, err) => function __init() {
    if (err) throw err[0];
    try {
      return fn && (res = (0, fn[__getOwnPropNames(fn)[0]])(fn = 0)), res;
    } catch (e) {
      throw err = [e], e;
    }
  };
  var __commonJS = (cb, mod) => function __require() {
    try {
      return mod || (0, cb[__getOwnPropNames(cb)[0]])((mod = { exports: {} }).exports, mod), mod.exports;
    } catch (e) {
      throw mod = 0, e;
    }
  };
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, { get: all[name], enumerable: true });
  };
  var __copyProps = (to, from, except, desc) => {
    if (from && typeof from === "object" || typeof from === "function") {
      for (let key of __getOwnPropNames(from))
        if (!__hasOwnProp.call(to, key) && key !== except)
          __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
    }
    return to;
  };
  var __reExport = (target, mod, secondTarget) => (__copyProps(target, mod, "default"), secondTarget && __copyProps(secondTarget, mod, "default"));
  var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
    // If the importer is in node compatibility mode or this is not an ESM
    // file that has been converted to a CommonJS file using a Babel-
    // compatible transform (i.e. "__esModule" has not been set), then set
    // "default" to the CommonJS "module.exports" for node compatibility.
    isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
    mod
  ));
  var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

  // <define:import.meta.env>
  var init_define_import_meta_env = __esm({
    "<define:import.meta.env>"() {
    }
  });

  // ds-raw:__ds_raw__
  var require_ds_raw = __commonJS({
    "ds-raw:__ds_raw__"(exports, module) {
      init_define_import_meta_env();
      module.exports = window.ScryerWeb;
    }
  });

  // shim:react-shim
  var require_react_shim = __commonJS({
    "shim:react-shim"(exports, module) {
      init_define_import_meta_env();
      var R = window.React;
      function jsx2(t, p, k) {
        return R.createElement(t, k === void 0 ? p : Object.assign({ key: k }, p));
      }
      module.exports = R;
      module.exports.jsx = jsx2;
      module.exports.jsxs = jsx2;
      module.exports.jsxDEV = jsx2;
      module.exports.Fragment = R.Fragment;
    }
  });

  // .design-sync/previews/Sidebar.tsx
  var Sidebar_exports = {};
  __export(Sidebar_exports, {
    Navigation: () => Navigation
  });
  init_define_import_meta_env();

  // ds-shim:ds
  var ds_exports = {};
  __export(ds_exports, {
    default: () => ds_default
  });
  init_define_import_meta_env();
  __reExport(ds_exports, __toESM(require_ds_raw()));
  var g = window.ScryerWeb;
  var ds_default = "default" in g ? g.default : g;

  // .design-sync/previews/Sidebar.tsx
  var import_jsx_runtime = __toESM(require_react_shim(), 1);
  var Icon = ({ d }) => /* @__PURE__ */ (0, import_jsx_runtime.jsx)("svg", { viewBox: "0 0 24 24", className: "size-4", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true, children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d }) });
  function Navigation() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarProvider, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Sidebar, { collapsible: "none", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarHeader, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "px-2 py-1.5 text-base font-semibold text-foreground", children: "Scryer" }) }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarContent, { children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarGroup, { children: [
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarGroupLabel, { children: "Library" }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarMenu, { children: [
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarMenuItem, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarMenuButton, { isActive: true, children: [
              /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Icon, { d: "m4 8 8-5 8 5v11a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1Z" }),
              "Movies"
            ] }) }),
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarMenuItem, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarMenuButton, { children: [
              /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Icon, { d: "M2 7h20v12H2zM8 3l4 4 4-4" }),
              "Series"
            ] }) }),
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarMenuItem, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarMenuButton, { children: [
              /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Icon, { d: "M8 2v4M16 2v4M3 10h18M5 4h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z" }),
              "Calendar"
            ] }) }),
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarMenuItem, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarMenuButton, { children: [
              /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Icon, { d: "M22 12h-4l-3 9L9 3l-3 9H2" }),
              "Activity"
            ] }) })
          ] })
        ] }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarGroup, { children: [
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarGroupLabel, { children: "System" }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarMenu, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.SidebarMenuItem, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.SidebarMenuButton, { children: [
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Icon, { d: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm8-3a8 8 0 0 0-.1-1l2-1.6-2-3.4-2.3 1a8 8 0 0 0-1.7-1l-.4-2.5H9.5L9 4.5a8 8 0 0 0-1.7 1l-2.3-1-2 3.4L5 9a8 8 0 0 0 0 2l-2 1.6 2 3.4 2.3-1a8 8 0 0 0 1.7 1l.4 2.5h4l.4-2.5a8 8 0 0 0 1.7-1l2.3 1 2-3.4L19 13c.1-.3.1-.7.1-1Z" }),
            "Settings"
          ] }) }) })
        ] })
      ] })
    ] }) });
  }
  return __toCommonJS(Sidebar_exports);
})();
