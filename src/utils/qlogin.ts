import { invoke } from "@tauri-apps/api/core";

export interface WebLoginStatus {
  status: "success" | "error" | "webLoginOpened" | "webLoginWaiting" | "webLoginCancelled";
  message: string;
}

export interface QzoneLoginUser {
  uin: string;
  nickname: string;
  avatarImage?: string;
}

export const openWebLogin = () => invoke<WebLoginStatus>("open_web_login");
export const checkWebLogin = () => invoke<WebLoginStatus>("check_web_login");
export const cancelWebLogin = () => invoke<void>("cancel_web_login");
export const getQzoneLoginUser = () => invoke<QzoneLoginUser>("get_qzone_login_user");
