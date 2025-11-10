require("test")

function TestCallsError()
	local ok, err = pcall(function()
		Check({})
	end)
	assert(not ok, "Expected Check({}) to raise an error")
	assert(string.find(err, "this is an error"), "Unexpected error message: " .. tostring(err))
end
