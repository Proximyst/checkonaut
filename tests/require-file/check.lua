require("example")

function Check(_)
	if RequiredFunction() == 1 then
		return {}
	end
	return { "should not happen" }
end
