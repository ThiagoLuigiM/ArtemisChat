import ReactDOM from "react-dom/client";
import FabWindow from "./windows/FabWindow";
import ChatWindow from "./windows/ChatWindow";
import "./styles.css";

const params = new URLSearchParams(window.location.search);
const windowType = params.get("window") ?? "chat";

document.documentElement.classList.add(`window-${windowType}`);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  windowType === "fab" ? <FabWindow /> : <ChatWindow />
);
