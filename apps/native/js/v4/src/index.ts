export { h, createElement, Fragment, createContext, Component } from "preact";
export type {
  ComponentChild,
  ComponentChildren,
  ComponentType,
  VNode,
} from "preact";
export { createRoot, SurfaceRoot, flushUpdates } from "./renderer";
export * from "./protocol";
export type { Selector, StyleRule } from "./styles";
