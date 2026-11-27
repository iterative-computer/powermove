import type { Evaluation } from "../../eval/harness";
import { createContext } from "powermove-v4";
import { useContext } from "powermove-v4/hooks";
import { Row, TextField } from "powermove-v4/ui";

export async function evaluate(e: Evaluation) {
  const Context = createContext("none");
  function Nested() {
    return (
      <TextField
        id="nested"
        value={useContext(Context)}
        binding="synthetic:nested"
      />
    );
  }
  await e.render(
    <Context.Provider value="exact">
      <Row label="Nested label">
        <Nested />
      </Row>
    </Context.Provider>,
  );
  e.props("nested", { value: "exact" });
  const label = e.node("nested").props.labelledBy;
  e.assert(
    typeof label === "string" && e.nodes().some((n) => n.props.id === label),
    "associated label exists",
  );
}
