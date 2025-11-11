local function hasName(obj)
	return obj.metadata and obj.metadata.name and string.len(obj.metadata.name) > 1
end

local function countContainers(obj)
	if not obj.spec or not obj.spec.containers then
		return 0
	end
	return #obj.spec.containers
end

local function allContainersHaveImages(obj)
	if not obj.spec or not obj.spec.containers then
		return false
	end
	for _, container in ipairs(obj.spec.containers) do
		if not container.image or string.len(container.image) == 0 then
			return false
		end
	end
	return true
end

function Check(obj)
	if string.lower(obj.apiVersion) ~= "v1" or string.lower(obj.kind) ~= "pod" then
		return nil
	end

	local issues = {}
	if not hasName(obj) then
		table.insert(issues, "pod is missing a name")
	end
	if countContainers(obj) < 1 then
		table.insert(issues, "pod has no container")
	end
	if not allContainersHaveImages(obj) then
		table.insert(issues, "one or more containers are missing an image")
	end
	return issues
end
